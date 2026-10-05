// iCloud 동기화 — 앱 쪽 절반(개발 10). CloudKit 은 헬퍼 앱(`sync/`, OuroSync.app)이 맡고, 이 모듈은 DB 와 헬퍼 사이를 잇는다.
//
// 설계 결정:
//   · **DB 는 앱만 쓴다.** 헬퍼는 표준입력·출력 JSON 줄로 «이 이름의 레코드 내용을 달라», «이걸 받았다» 만 말한다(sync/Sources/OuroSyncKit/Messages.swift).
//   · **무엇이 바뀌었나는 트리거가 적는다**(스키마 9, `sync_outbox`). 팝오버·MCP·디스패처 어디서 쓰든 빠지지 않는다.
//     받은 것을 적는 동안은 `applying` 깃발로 트리거를 멈춘다 — 받은 걸 도로 보내는 메아리가 없게.
//   · 레코드 이름 = `uid`(스키마 7). 정수 id 를 가리키는 칸(부탁의 이어서·반복·실행의 부탁 등)은 **uid 로 바꿔 싣고**, 받을 때 이 기기의 id 로 푼다.
//     아직 못 푸는 참조(부탁보다 실행이 먼저 온 경우 등)는 `sync_inbox` 에 세워 두고 다음 묶음 뒤에 다시 푼다.
//   · 칸은 둘로 나눈다: 그냥 칸(시각·상태)과 **비밀 칸**(제목·메모·부탁 글·답 — CloudKit `encryptedValues`, 종단간 암호화, PLAN §13).
//   · **충돌은 앱이 정한다.** 일정·부탁 = 늦게 고친 쪽(`updated_at`). 실행 = 끝난 쪽(도는 중 < 끝남), 읽음은 어느 쪽이든 읽었으면 읽음.
//     제안 = 먼저 결정된 쪽. 내 쪽이 이기면 다시 보낸다.
//   · **실행 맥**: 동기화가 켜지면 예약 부탁은 «실행 맥» 한 대만 돌린다(PLAN §13 — 이중 실행 방지). 설정은 `Config/runner` 레코드로 오간다.
//     처음 켠 맥은 첫 받기가 끝난 뒤에도 실행 맥이 없을 때만 스스로 맡는다. 수동 «지금 실행» 은 어느 맥에서나 된다(사람이 그 맥 앞에 있다).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::store::{now_ms, Store};

/// 헬퍼와 주고받는 레코드 — `WireRecord`(Swift)와 같은 모양.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    #[serde(rename = "type")]
    pub ty: String,
    pub name: String,
    #[serde(default)]
    pub fields: Map<String, Value>,
    #[serde(default)]
    pub secret: Map<String, Value>,
    #[serde(default)]
    pub system: Option<String>,
}

impl Record {
    /// 그냥 칸 + 비밀 칸을 한 지도로.
    fn all(&self) -> Map<String, Value> {
        let mut m = self.fields.clone();
        m.extend(self.secret.clone());
        m
    }
}

/// 다른 표의 줄을 가리키는 칸: 레코드에선 `field` = 상대의 uid, DB 에선 `col` = 상대의 정수 id.
struct Ref {
    field: &'static str,
    col: &'static str,
    table: &'static str,
}

/// 레코드 종류 하나 = 표 하나의 칸 목록.
struct Spec {
    ty: &'static str,
    table: &'static str,
    plain: &'static [&'static str],
    secret: &'static [&'static str],
    refs: &'static [Ref],
}

const ITEM: Spec = Spec {
    ty: "Item",
    table: "items",
    plain: &[
        "kind", "all_day", "start_at", "end_at", "start_date", "end_date", "tz", "rrule", "alert_min", "private", "origin",
        "created_at", "updated_at", "deleted_at",
    ],
    secret: &["title", "notes"],
    refs: &[],
};
/// 부탁 칸 — `items` 레코드에 같이 싣는다(부탁은 items 한 줄 + errands 한 줄이라, 따로 보내면 반쪽만 도착하는 때가 생긴다).
const ERRAND: Spec = Spec {
    ty: "Item",
    table: "errands",
    plain: &["target", "allowed_tools", "approved_at", "depth", "late", "carry", "wall_time"],
    secret: &["prompt", "workdir", "resume_session"],
    refs: &[
        Ref { field: "parent_run", col: "parent_run_id", table: "runs" },
        Ref { field: "resume_run", col: "resume_run_id", table: "runs" },
        Ref { field: "series", col: "series_id", table: "items" },
        Ref { field: "folder", col: "folder_id", table: "items" },
    ],
};
const RUN: Spec = Spec {
    ty: "Run",
    table: "runs",
    plain: &["started_at", "finished_at", "exit_code", "status", "late_ms", "read_at", "device"],
    secret: &["sent_text", "response", "stderr", "session_id", "resumed_session"],
    refs: &[Ref { field: "item", col: "item_id", table: "items" }],
};
const PROPOSAL: Spec = Spec {
    ty: "Proposal",
    table: "proposals",
    plain: &["all_day", "start_at", "end_at", "start_date", "end_date", "alert_min", "status", "created_at", "decided_at"],
    secret: &["client", "title", "notes"],
    refs: &[Ref { field: "event", col: "event_id", table: "items" }],
};
const ERRAND_PROPOSAL: Spec = Spec {
    ty: "ErrandProposal",
    table: "errand_proposals",
    plain: &["start_at", "allowed_tools", "late", "depth", "status", "created_at", "decided_at", "target"],
    secret: &["client", "prompt"],
    refs: &[
        Ref { field: "parent_run", col: "parent_run_id", table: "runs" },
        Ref { field: "errand", col: "errand_id", table: "items" },
    ],
};
/// 정수 id 로 된 표들(`Config` 는 표가 아니라 설정 몇 칸).
const TABLES: [&Spec; 4] = [&ITEM, &RUN, &PROPOSAL, &ERRAND_PROPOSAL];

const CONFIG_TYPE: &str = "Config";
const RUNNER: &str = "runner";

fn spec_of_table(t: &str) -> Option<&'static Spec> {
    TABLES.iter().copied().find(|s| s.table == t)
}
fn spec_of_type(t: &str) -> Option<&'static Spec> {
    TABLES.iter().copied().find(|s| s.ty == t)
}

fn sql_value(v: ValueRef) -> Value {
    match v {
        ValueRef::Null | ValueRef::Blob(_) => Value::Null,
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => Value::from(f),
        ValueRef::Text(t) => Value::from(String::from_utf8_lossy(t).into_owned()),
    }
}

/// JSON 값 → SQL 인자. 불리언은 0/1(Swift 쪽도 그렇게 바꾼다).
fn to_sql(v: Option<&Value>) -> rusqlite::types::Value {
    use rusqlite::types::Value as S;
    match v {
        None | Some(Value::Null) => S::Null,
        Some(Value::Bool(b)) => S::Integer(*b as i64),
        Some(Value::Number(n)) => n.as_i64().map(S::Integer).unwrap_or_else(|| S::Real(n.as_f64().unwrap_or(0.0))),
        Some(Value::String(s)) => S::Text(s.clone()),
        Some(other) => S::Text(other.to_string()),
    }
}

fn int(m: &Map<String, Value>, k: &str) -> Option<i64> {
    m.get(k).and_then(Value::as_i64)
}
fn text<'a>(m: &'a Map<String, Value>, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

/// 한 표의 한 줄을 지도로(참조 칸은 상대 uid 로). 없으면 None.
fn read_row(conn: &Connection, s: &Spec, key_col: &str, key: &dyn rusqlite::ToSql) -> rusqlite::Result<Option<(i64, Map<String, Value>)>> {
    let id_col = if s.table == "errands" { "item_id" } else { "id" };
    let mut cols: Vec<String> = s.plain.iter().chain(s.secret).map(|c| format!("t.{c}")).collect();
    for r in s.refs {
        cols.push(format!("(SELECT uid FROM {} WHERE id = t.{})", r.table, r.col));
    }
    let sql = format!("SELECT t.{id_col}, {} FROM {} t WHERE t.{key_col} = ?1", cols.join(", "), s.table);
    conn.query_row(&sql, [key], |row| {
        let mut m = Map::new();
        let names = s.plain.iter().chain(s.secret).chain(s.refs.iter().map(|r| &r.field));
        for (i, n) in names.enumerate() {
            m.insert((*n).to_string(), sql_value(row.get_ref(i + 1)?));
        }
        Ok((row.get(0)?, m))
    })
    .optional()
}

fn errand_keys() -> impl Iterator<Item = &'static str> {
    ERRAND.plain.iter().chain(ERRAND.secret).copied().chain(ERRAND.refs.iter().map(|r| r.field))
}

/// 이 기기의 레코드 하나(내용 + 지난번 서버 판의 시스템 칸). 없으면 None.
pub(crate) fn build(conn: &Connection, ty: &str, name: &str) -> rusqlite::Result<Option<Record>> {
    let all = if ty == CONFIG_TYPE {
        if name != RUNNER {
            return Ok(None);
        }
        let Some(device) = setting(conn, "sync_runner")? else { return Ok(None) };
        let mut m = Map::new();
        m.insert("device".into(), device.into());
        m.insert("name".into(), setting(conn, "sync_runner_name")?.unwrap_or_default().into());
        m.insert("at".into(), setting(conn, "sync_runner_at")?.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0).into());
        m
    } else {
        let Some(s) = spec_of_type(ty) else { return Ok(None) };
        let Some((id, mut m)) = read_row(conn, s, "uid", &name)? else { return Ok(None) };
        if s.ty == "Item" {
            match read_row(conn, &ERRAND, "item_id", &id)? {
                Some((_, e)) => m.extend(e),
                None => {
                    for k in errand_keys() {
                        m.insert(k.into(), Value::Null);
                    }
                }
            }
        }
        if s.ty == "Run" && m.get("device").is_none_or(Value::is_null) {
            // 스키마 9 전의 실행 — 이 기기가 돌린 것이다.
            m.insert("device".into(), device_id(conn)?.into());
        }
        m
    };
    Ok(Some(split(ty, name, all, meta(conn, name)?)))
}

/// 지도를 레코드로 — 어느 칸이 비밀인지는 종류의 목록이 정한다.
fn split(ty: &str, name: &str, all: Map<String, Value>, system: Option<String>) -> Record {
    let secret_keys: Vec<&str> = match ty {
        "Item" => ITEM.secret.iter().chain(ERRAND.secret).copied().collect(),
        _ => spec_of_type(ty).map(|s| s.secret.to_vec()).unwrap_or_default(),
    };
    let (mut fields, mut secret) = (Map::new(), Map::new());
    for (k, v) in all {
        if secret_keys.contains(&k.as_str()) {
            secret.insert(k, v);
        } else {
            fields.insert(k, v);
        }
    }
    Record { ty: ty.into(), name: name.into(), fields, secret, system }
}

fn setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()
}
fn put_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map(|_| ())
}
pub(crate) fn device_id(conn: &Connection) -> rusqlite::Result<String> {
    setting(conn, "device_id").map(|v| v.unwrap_or_default())
}
fn meta(conn: &Connection, name: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT system FROM sync_meta WHERE name = ?1", [name], |r| r.get(0)).optional()
}
fn flag(conn: &Connection, k: &str) -> rusqlite::Result<i64> {
    conn.query_row("SELECT v FROM sync_flags WHERE k = ?1", [k], |r| r.get(0))
}
fn set_flag(conn: &Connection, k: &str, v: i64) -> rusqlite::Result<()> {
    conn.execute("UPDATE sync_flags SET v = ?2 WHERE k = ?1", params![k, v]).map(|_| ())
}

/// 이 맥이 예약 부탁을 돌려도 되나. 동기화가 꺼져 있으면 언제나 된다(혼자 쓰는 맥).
/// 켜져 있으면 실행 맥으로 지정된 맥만 — 아직 아무도 안 맡았으면 아무도 안 돌린다(첫 받기가 끝나면 누군가 맡는다).
pub(crate) fn is_runner(conn: &Connection) -> rusqlite::Result<bool> {
    if flag(conn, "on")? == 0 {
        return Ok(true);
    }
    Ok(setting(conn, "sync_runner")?.is_some_and(|r| r == device_id(conn).unwrap_or_default()))
}

/// 보낼 목록에 «저장» 한 줄을 직접 넣는다(트리거가 꺼진 `applying` 중에 «내 쪽이 이겼다» 를 적을 때, 설정 레코드).
fn enqueue_save(conn: &Connection, key: &str, tbl: &str, rid: Option<i64>) -> rusqlite::Result<()> {
    conn.execute("UPDATE sync_flags SET v = v + 1 WHERE k = 'seq'", [])?;
    conn.execute(
        "INSERT INTO sync_outbox (key, tbl, rid, uid, op, seq) VALUES (?1, ?2, ?3, NULL, 'save', (SELECT v FROM sync_flags WHERE k = 'seq'))
         ON CONFLICT (key) DO UPDATE SET op = 'save', seq = excluded.seq",
        params![key, tbl, rid],
    )?;
    Ok(())
}

/// 동기화를 처음 켤 때(또는 서버 데이터가 초기화됐을 때) 가진 것을 전부 보낼 목록에.
fn enqueue_all(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("UPDATE sync_flags SET v = v + 1 WHERE k = 'seq'", [])?;
    for s in TABLES {
        conn.execute(
            &format!(
                "INSERT INTO sync_outbox (key, tbl, rid, uid, op, seq)
                 SELECT '{t}:' || id, '{t}', id, NULL, 'save', (SELECT v FROM sync_flags WHERE k = 'seq') FROM {t} WHERE true
                 ON CONFLICT (key) DO UPDATE SET op = 'save', seq = excluded.seq",
                t = s.table
            ),
            [],
        )?;
    }
    if setting(conn, "sync_runner")?.is_some() {
        enqueue_save(conn, "config:runner", "config", None)?;
    }
    Ok(())
}

/// 보낼 목록 한 줄을 헬퍼가 알아듣는 (종류, 이름, 저장인가)로.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pending {
    pub key: String,
    pub ty: String,
    pub name: String,
    pub save: bool,
    pub seq: i64,
}

/// `seq` 뒤로 바뀐 보낼 목록. 저장인데 줄이 이미 없으면(지워짐 — 삭제 줄이 따로 있다) 목록에서 걷어 낸다.
pub(crate) fn outbox_since(conn: &Connection, seq: i64) -> rusqlite::Result<Vec<Pending>> {
    type Row = (String, String, Option<i64>, Option<String>, String, i64);
    let rows: Vec<Row> = {
        let mut st = conn.prepare("SELECT key, tbl, rid, uid, op, seq FROM sync_outbox WHERE seq > ?1 ORDER BY seq")?;
        let it = st.query_map([seq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?;
        it.collect::<rusqlite::Result<_>>()?
    };
    let mut out = vec![];
    for (key, tbl, rid, uid, op, seq) in rows {
        if tbl == "config" {
            out.push(Pending { key, ty: CONFIG_TYPE.into(), name: RUNNER.into(), save: true, seq });
            continue;
        }
        let Some(s) = spec_of_table(&tbl) else { continue };
        if op == "delete" {
            if let Some(u) = uid {
                out.push(Pending { key, ty: s.ty.into(), name: u, save: false, seq });
            }
            continue;
        }
        let uid: Option<Option<String>> =
            conn.query_row(&format!("SELECT uid FROM {} WHERE id = ?1", s.table), [rid], |r| r.get(0)).optional()?;
        match uid {
            Some(Some(u)) => out.push(Pending { key, ty: s.ty.into(), name: u, save: true, seq }),
            Some(None) => {} // uid 채우는 트리거가 곧 채운다 — 그 UPDATE 가 seq 를 다시 올린다.
            None => {
                conn.execute("DELETE FROM sync_outbox WHERE key = ?1", [&key])?;
            }
        }
    }
    Ok(out)
}

/// 이름(uid)으로 이 기기의 줄을 찾는다 → (종류, 보낼 목록 키).
fn locate(conn: &Connection, name: &str) -> rusqlite::Result<Option<(&'static str, String)>> {
    if name == RUNNER {
        return Ok(Some((CONFIG_TYPE, "config:runner".into())));
    }
    for s in TABLES {
        let id: Option<i64> =
            conn.query_row(&format!("SELECT id FROM {} WHERE uid = ?1", s.table), [name], |r| r.get(0)).optional()?;
        if let Some(id) = id {
            return Ok(Some((s.ty, format!("{}:{id}", s.table))));
        }
    }
    Ok(None)
}

// ── 받은 것 적기 ─────────────────────────────────────────────

/// 같은 값인가 — 없는 칸과 null 은 같다.
fn same(a: &Map<String, Value>, b: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().all(|k| a.get(*k).unwrap_or(&Value::Null) == b.get(*k).unwrap_or(&Value::Null))
}

fn keys_of(ty: &str) -> Vec<&'static str> {
    match ty {
        CONFIG_TYPE => vec!["device", "name", "at"],
        "Item" => ITEM.plain.iter().chain(ITEM.secret).copied().chain(errand_keys()).collect(),
        _ => spec_of_type(ty)
            .map(|s| s.plain.iter().chain(s.secret).copied().chain(s.refs.iter().map(|r| r.field)).collect())
            .unwrap_or_default(),
    }
}

/// 둘 다 가진 레코드를 합친다(순수 — 테스트 가능). 어느 판을 바탕으로 할지 + 실행의 «읽음».
pub(crate) fn merge(ty: &str, local: &Map<String, Value>, remote: &Map<String, Value>) -> Map<String, Value> {
    match ty {
        "Item" => {
            // 늦게 고친 쪽. 같으면 서버 판 — 두 기기가 같은 결론에 닿게.
            if int(local, "updated_at").unwrap_or(0) > int(remote, "updated_at").unwrap_or(0) {
                local.clone()
            } else {
                remote.clone()
            }
        }
        CONFIG_TYPE => {
            if int(local, "at").unwrap_or(0) > int(remote, "at").unwrap_or(0) {
                local.clone()
            } else {
                remote.clone()
            }
        }
        "Run" => {
            // 도는 중 < 끝남. 둘 다 끝났으면 서버 판(한 실행은 한 맥만 끝낸다 — 갈릴 일이 거의 없다).
            let lf = local.get("finished_at").is_some_and(|v| !v.is_null());
            let rf = remote.get("finished_at").is_some_and(|v| !v.is_null());
            let mut m = if lf && !rf { local.clone() } else { remote.clone() };
            // 읽음은 어느 쪽에서든 읽었으면 읽음 — 이른 시각으로.
            let read = [int(local, "read_at"), int(remote, "read_at")].into_iter().flatten().min();
            m.insert("read_at".into(), read.map_or(Value::Null, Value::from));
            m
        }
        _ => {
            // 제안: 결정은 되돌아가지 않는다. 둘 다 결정했는데 다르면 먼저 결정한 쪽.
            let ls = text(local, "status").unwrap_or("pending");
            let rs = text(remote, "status").unwrap_or("pending");
            if ls != "pending" && rs == "pending" {
                return local.clone();
            }
            if ls != "pending" && rs != "pending" && !same(local, remote, &["status", "decided_at"]) {
                let (ld, rd) = (int(local, "decided_at").unwrap_or(i64::MAX), int(remote, "decided_at").unwrap_or(i64::MAX));
                if ld < rd {
                    return local.clone();
                }
            }
            remote.clone()
        }
    }
}

#[derive(Debug, PartialEq)]
enum Applied {
    Done,
    /// 아직 없는 줄을 가리킨다 — 세워 두고 나중에.
    Park(String),
}

/// 참조 칸(uid)을 이 기기의 id 로. 비었으면 None, 못 찾으면 Err(세워 둘 이유).
fn resolve(conn: &Connection, r: &Ref, m: &Map<String, Value>) -> Result<Option<i64>, String> {
    match m.get(r.field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(u)) => conn
            .query_row(&format!("SELECT id FROM {} WHERE uid = ?1", r.table), [u], |x| x.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .map(Some)
            .ok_or_else(|| format!("{} {u} 가 아직 없어요", r.field)),
        Some(_) => Err(format!("{} 칸 모양이 이상해요", r.field)),
    }
}

/// 지도를 한 표에 쓴다. `id` 가 있으면 고치고, 없으면 `uid` 로 새 줄.
fn write_row(conn: &Connection, s: &Spec, uid: &str, id: Option<i64>, m: &Map<String, Value>) -> Result<i64, String> {
    let mut cols: Vec<&str> = s.plain.iter().chain(s.secret).copied().collect();
    let mut vals: Vec<rusqlite::types::Value> = cols.iter().map(|c| to_sql(m.get(*c))).collect();
    for r in s.refs {
        cols.push(r.col);
        vals.push(resolve(conn, r, m)?.map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Integer));
    }
    match id {
        Some(id) => {
            let set: Vec<String> = cols.iter().enumerate().map(|(i, c)| format!("{c} = ?{}", i + 2)).collect();
            let mut args: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Integer(id)];
            args.extend(vals);
            conn.execute(&format!("UPDATE {} SET {} WHERE id = ?1", s.table, set.join(", ")), rusqlite::params_from_iter(args))
                .map_err(|e| e.to_string())?;
            Ok(id)
        }
        None => {
            let marks: Vec<String> = (0..=cols.len()).map(|i| format!("?{}", i + 1)).collect();
            let mut args: Vec<rusqlite::types::Value> = vec![rusqlite::types::Value::Text(uid.into())];
            args.extend(vals);
            conn.execute(
                &format!("INSERT INTO {} (uid, {}) VALUES ({})", s.table, cols.join(", "), marks.join(", ")),
                rusqlite::params_from_iter(args),
            )
            .map_err(|e| e.to_string())?;
            Ok(conn.last_insert_rowid())
        }
    }
}

/// 부탁 칸 쓰기(item 이 errand 일 때). 참조가 자기 자신(반복의 첫 회차)이어도 풀리게 items 줄을 먼저 쓴 뒤 부른다.
fn write_errand(conn: &Connection, item_id: i64, m: &Map<String, Value>) -> Result<(), String> {
    let mut cols: Vec<&str> = ERRAND.plain.iter().chain(ERRAND.secret).copied().collect();
    let mut vals: Vec<rusqlite::types::Value> = cols.iter().map(|c| to_sql(m.get(*c))).collect();
    for r in ERRAND.refs {
        cols.push(r.col);
        vals.push(resolve(conn, r, m)?.map_or(rusqlite::types::Value::Null, rusqlite::types::Value::Integer));
    }
    let marks: Vec<String> = (0..=cols.len()).map(|i| format!("?{}", i + 1)).collect();
    let set: Vec<String> = cols.iter().map(|c| format!("{c} = excluded.{c}")).collect();
    let mut args = vec![rusqlite::types::Value::Integer(item_id)];
    args.extend(vals);
    conn.execute(
        &format!(
            "INSERT INTO errands (item_id, {}) VALUES ({}) ON CONFLICT (item_id) DO UPDATE SET {}",
            cols.join(", "),
            marks.join(", "),
            set.join(", ")
        ),
        rusqlite::params_from_iter(args),
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 받은 레코드 하나를 적는다(트랜잭션·`applying` 안에서 부른다). 내 판과 합쳐서, 합친 결과가 서버 판과 다르면 다시 보낸다.
fn apply_one(conn: &Connection, rec: &Record) -> Result<Applied, String> {
    let remote = rec.all();
    let local = build(conn, &rec.ty, &rec.name).map_err(|e| e.to_string())?;
    let merged = match &local {
        Some(l) => merge(&rec.ty, &l.all(), &remote),
        None => remote.clone(),
    };
    let keys = keys_of(&rec.ty);
    if keys.is_empty() {
        return Ok(Applied::Done); // 모르는 종류(더 새 버전이 만든 것) — 건드리지 않는다.
    }
    let changed_here = local.as_ref().is_none_or(|l| !same(&l.all(), &merged, &keys));
    if changed_here {
        if rec.ty == CONFIG_TYPE {
            if rec.name == RUNNER {
                put_setting(conn, "sync_runner", text(&merged, "device").unwrap_or_default()).map_err(|e| e.to_string())?;
                put_setting(conn, "sync_runner_name", text(&merged, "name").unwrap_or_default()).map_err(|e| e.to_string())?;
                put_setting(conn, "sync_runner_at", &int(&merged, "at").unwrap_or(0).to_string()).map_err(|e| e.to_string())?;
            }
        } else {
            let s = spec_of_type(&rec.ty).expect("keys_of 가 걸렀다");
            let id: Option<i64> = conn
                .query_row(&format!("SELECT id FROM {} WHERE uid = ?1", s.table), [&rec.name], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            let id = match write_row(conn, s, &rec.name, id, &merged) {
                Ok(id) => id,
                Err(e) if e.contains("아직 없어요") => return Ok(Applied::Park(e)),
                Err(e) => return Err(e),
            };
            if s.ty == "Item" {
                if text(&merged, "kind") == Some("errand") {
                    if let Err(e) = write_errand(conn, id, &merged) {
                        return if e.contains("아직 없어요") { Ok(Applied::Park(e)) } else { Err(e) };
                    }
                } else {
                    conn.execute("DELETE FROM errands WHERE item_id = ?1", [id]).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    if let Some(sys) = &rec.system {
        conn.execute(
            "INSERT INTO sync_meta (name, system) VALUES (?1, ?2) ON CONFLICT (name) DO UPDATE SET system = excluded.system",
            params![rec.name, sys],
        )
        .map_err(|e| e.to_string())?;
    }
    let (_, key) = locate(conn, &rec.name).map_err(|e| e.to_string())?.ok_or("방금 적은 줄을 못 찾았어요")?;
    if same(&merged, &remote, &keys) {
        // 서버 판이 그대로 이겼다 — 이 기기의 보내지 못한 옛 변경은 진 것이다.
        conn.execute("DELETE FROM sync_outbox WHERE key = ?1", [&key]).map_err(|e| e.to_string())?;
    } else {
        let (tbl, rid) = key.split_once(':').map(|(t, r)| (t.to_string(), r.parse::<i64>().ok())).unwrap_or_default();
        enqueue_save(conn, &key, &tbl, rid).map_err(|e| e.to_string())?;
    }
    Ok(Applied::Done)
}

/// 서버에서 지워진 레코드. 실행 기록이 남은 부탁은 지우지 않는다 — 바깥으로 나간 원문을 지키고(CLAUDE.md 불변 규칙),
/// 서버엔 다시 올린다(다른 기기가 그 실행을 아직 못 받았을 수 있다).
fn apply_delete(conn: &Connection, ty: &str, name: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sync_inbox WHERE name = ?1", [name])?;
    conn.execute("DELETE FROM sync_meta WHERE name = ?1", [name])?;
    if ty == CONFIG_TYPE {
        return Ok(());
    }
    let Some(s) = spec_of_type(ty) else { return Ok(()) };
    let Some(id): Option<i64> = conn.query_row(&format!("SELECT id FROM {} WHERE uid = ?1", s.table), [name], |r| r.get(0)).optional()?
    else {
        return Ok(());
    };
    let key = format!("{}:{id}", s.table);
    if s.ty == "Item" {
        let has_runs: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM runs WHERE item_id = ?1)", [id], |r| r.get(0))?;
        if has_runs {
            return enqueue_save(conn, &key, s.table, Some(id));
        }
    }
    conn.execute("DELETE FROM sync_outbox WHERE key = ?1", [&key])?;
    conn.execute(&format!("DELETE FROM {} WHERE id = ?1", s.table), [id])?;
    Ok(())
}

/// 받은 묶음을 한 트랜잭션으로 적는다. 세워 둔 것도 다시 풀어 본다. 돌려주는 값 = 바뀐 게 있었나.
pub(crate) fn apply(store: &Store, records: &[Record], deleted: &[(String, String)]) -> Result<bool, String> {
    let mut conn = store.conn();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    set_flag(&tx, "applying", 1).map_err(|e| e.to_string())?;
    let mut todo: Vec<Record> = records.to_vec();
    let parked: Vec<(String, String)> = {
        let mut st = tx.prepare("SELECT name, payload FROM sync_inbox").map_err(|e| e.to_string())?;
        let it = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        it.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?
    };
    for (name, payload) in parked {
        // 같은 이름의 새 판이 이번 묶음에 있으면 옛 판은 버린다.
        if !todo.iter().any(|r| r.name == name) {
            if let Ok(r) = serde_json::from_str::<Record>(&payload) {
                todo.push(r);
            }
        }
    }
    for (ty, name) in deleted {
        apply_delete(&tx, ty, name).map_err(|e| e.to_string())?;
        todo.retain(|r| &r.name != name);
    }
    // 참조가 풀릴 때까지 여러 바퀴 — 실행이 부탁보다 먼저 와도 같은 묶음 안이면 둘째 바퀴에 풀린다.
    let mut changed = !deleted.is_empty();
    loop {
        let mut progress = false;
        let mut left = vec![];
        for r in todo {
            tx.execute_batch("SAVEPOINT one").map_err(|e| e.to_string())?;
            match apply_one(&tx, &r) {
                Ok(Applied::Done) => {
                    tx.execute_batch("RELEASE one").map_err(|e| e.to_string())?;
                    tx.execute("DELETE FROM sync_inbox WHERE name = ?1", [&r.name]).map_err(|e| e.to_string())?;
                    progress = true;
                }
                Ok(Applied::Park(why)) | Err(why) => {
                    tx.execute_batch("ROLLBACK TO one; RELEASE one").map_err(|e| e.to_string())?;
                    left.push((r, why));
                }
            }
        }
        changed |= progress;
        if left.is_empty() || !progress {
            for (r, why) in &left {
                eprintln!("ouro: 동기화 레코드를 세워 둠 {} {} — {why}", r.ty, r.name);
                tx.execute(
                    "INSERT INTO sync_inbox (name, type, payload, why, at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (name) DO UPDATE SET type = excluded.type, payload = excluded.payload, why = excluded.why, at = excluded.at",
                    params![r.name, r.ty, serde_json::to_string(r).unwrap_or_default(), why, now_ms()],
                )
                .map_err(|e| e.to_string())?;
            }
            break;
        }
        todo = left.into_iter().map(|(r, _)| r).collect();
    }
    set_flag(&tx, "applying", 0).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

// ── 켜고 끄기 · 실행 맥 ──────────────────────────────────────

/// 동기화 켜기/끄기의 DB 쪽. 켤 때 가진 것을 전부 보낼 목록에 넣는다. 끌 때 보낼 목록은 비운다(다시 켜면 다시 전부).
pub(crate) fn set_on(store: &Store, on: bool) -> Result<(), String> {
    let mut conn = store.conn();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if on {
        let me = device_id(&tx).map_err(|e| e.to_string())?;
        tx.execute("UPDATE runs SET device = ?1 WHERE device IS NULL", [&me]).map_err(|e| e.to_string())?;
        set_flag(&tx, "on", 1).map_err(|e| e.to_string())?;
        enqueue_all(&tx).map_err(|e| e.to_string())?;
    } else {
        set_flag(&tx, "on", 0).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sync_outbox", []).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// 이 맥을 실행 맥으로. `only_if_none` = 아직 아무도 안 맡았을 때만(처음 켠 맥이 첫 받기 뒤에 부른다).
pub(crate) fn claim_runner(store: &Store, name: &str, only_if_none: bool) -> Result<bool, String> {
    let conn = store.conn();
    if flag(&conn, "on").map_err(|e| e.to_string())? == 0 {
        return Ok(false);
    }
    let cur = setting(&conn, "sync_runner").map_err(|e| e.to_string())?;
    if only_if_none && cur.as_deref().is_some_and(|c| !c.is_empty()) {
        return Ok(false);
    }
    let me = device_id(&conn).map_err(|e| e.to_string())?;
    put_setting(&conn, "sync_runner", &me).map_err(|e| e.to_string())?;
    put_setting(&conn, "sync_runner_name", name).map_err(|e| e.to_string())?;
    put_setting(&conn, "sync_runner_at", &now_ms().to_string()).map_err(|e| e.to_string())?;
    enqueue_save(&conn, "config:runner", "config", None).map_err(|e| e.to_string())?;
    Ok(true)
}

/// 서버의 내 데이터를 더는 믿을 수 없을 때(계정이 바뀜·존이 지워짐) — 시스템 칸을 버린다.
fn forget_server(store: &Store) -> Result<(), String> {
    let conn = store.conn();
    conn.execute_batch("DELETE FROM sync_meta; DELETE FROM sync_inbox;").map_err(|e| e.to_string())
}

// ── 헬퍼 프로세스 ────────────────────────────────────────────

/// 팝오버에 보이는 동기화 상태.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncStatus {
    /// 이 앱에 헬퍼가 들어 있나(없으면 동기화 줄을 아예 안 보인다).
    pub available: bool,
    pub on: bool,
    /// iCloud 계정 상태(헬퍼가 알려 준 것). available · noAccount · restricted · temporarilyUnavailable …
    pub account: Option<String>,
    pub last_sync: Option<i64>,
    pub error: Option<String>,
    pub pending: i64,
    pub runner_name: Option<String>,
    pub runner_is_me: bool,
    pub this_name: String,
}

type OnChange = Arc<dyn Fn(Change) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Change {
    /// 받은 것을 적었다 — 목록을 다시 읽고 디스패처를 깨운다.
    Data,
    /// 상태 줄만.
    Status,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
    /// 프로세스마다 번호 — 늦게 끝난 옛 리더가 새 헬퍼의 자리를 지우지 않게.
    gen: u64,
}

struct Inner {
    store: Arc<Store>,
    state_dir: PathBuf,
    helper: Option<PathBuf>,
    this_name: String,
    child: Mutex<Option<Running>>,
    gen: Mutex<u64>,
    /// 헬퍼에 알린 보낼 목록의 마지막 seq. 헬퍼를 새로 띄우면 0 — 처음부터 다시 알린다.
    announced: Mutex<i64>,
    /// 헬퍼가 내용을 가져간 이름 → (보낼 목록 키, 그때의 seq). 저장이 끝나면 그 seq 까지만 지운다(그사이 또 고친 건 남는다).
    inflight: Mutex<HashMap<String, (String, i64)>>,
    status: Mutex<SyncStatus>,
    on_change: OnChange,
    /// 헬퍼가 죽었을 때 다시 띄울 시각(연달아 죽으면 계속 띄우지 않게).
    retry_at: Mutex<Option<Instant>>,
}

#[derive(Clone)]
pub(crate) struct Syncer {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// 헬퍼 위치: 배포본 = `Ouro.app/Contents/Helpers/OuroSync.app`. 디버그 빌드만 `OURO_SYNC_HELPER` 로 바꿀 수 있다.
pub(crate) fn find_helper() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("OURO_SYNC_HELPER") {
        return Some(PathBuf::from(p)).filter(|p| p.is_file());
    }
    let exe = std::env::current_exe().ok()?;
    let p = exe.parent()?.parent()?.join("Helpers/OuroSync.app/Contents/MacOS/OuroSync");
    p.is_file().then_some(p)
}

/// 이 맥의 이름(«태운의 MacBook Pro»). 실행 맥을 사람에게 보일 때 쓴다.
pub(crate) fn computer_name() -> String {
    Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "이 맥".into())
}

const TICK: Duration = Duration::from_secs(2);
const FETCH_EVERY: Duration = Duration::from_secs(2 * 60);
const RESTART_AFTER: Duration = Duration::from_secs(30);

impl Syncer {
    pub(crate) fn start(store: Arc<Store>, dir: &Path, helper: Option<PathBuf>, this_name: String, on_change: OnChange) -> Syncer {
        let inner = Arc::new(Inner {
            store,
            state_dir: dir.join("sync"),
            helper,
            this_name,
            child: Mutex::default(),
            gen: Mutex::default(),
            announced: Mutex::default(),
            inflight: Mutex::default(),
            status: Mutex::default(),
            on_change,
            retry_at: Mutex::default(),
        });
        let s = Syncer { inner };
        s.refresh_status();
        if s.is_on() {
            s.spawn();
        }
        let t = s.clone();
        std::thread::Builder::new()
            .name("ouro-sync".into())
            .spawn(move || {
                let mut last_fetch = Instant::now();
                loop {
                    std::thread::sleep(TICK);
                    t.tick(&mut last_fetch);
                }
            })
            .expect("동기화 스레드를 띄우지 못했어요");
        s
    }

    fn is_on(&self) -> bool {
        flag(&self.inner.store.conn(), "on").unwrap_or(0) == 1
    }

    fn tick(&self, last_fetch: &mut Instant) {
        if !self.is_on() || self.inner.helper.is_none() {
            return;
        }
        if lock(&self.inner.child).is_none() {
            let due = lock(&self.inner.retry_at).is_none_or(|t| Instant::now() >= t);
            if due {
                self.spawn();
            }
            return;
        }
        self.announce();
        if last_fetch.elapsed() >= FETCH_EVERY {
            *last_fetch = Instant::now();
            self.send(&serde_json::json!({"op": "fetch"}));
        }
    }

    /// 보낼 목록의 새 줄을 헬퍼에 알린다.
    fn announce(&self) {
        let since = *lock(&self.inner.announced);
        let rows = match outbox_since(&self.inner.store.conn(), since) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ouro: 보낼 목록 읽기 실패 — {e}");
                return;
            }
        };
        let Some(max) = rows.iter().map(|p| p.seq).max() else { return };
        let refs = |save: bool| -> Vec<Value> {
            rows.iter().filter(|p| p.save == save).map(|p| serde_json::json!({"type": p.ty, "name": p.name})).collect()
        };
        if self.send(&serde_json::json!({"op": "pending", "save": refs(true), "delete": refs(false)})) {
            *lock(&self.inner.announced) = max;
        }
        self.refresh_status();
    }

    fn send(&self, v: &Value) -> bool {
        let mut g = lock(&self.inner.child);
        let Some(r) = g.as_mut() else { return false };
        writeln!(r.stdin, "{v}").and_then(|_| r.stdin.flush()).is_ok()
    }

    fn spawn(&self) {
        let Some(helper) = self.inner.helper.clone() else { return };
        let mut g = lock(&self.inner.child);
        if g.is_some() {
            return;
        }
        let _ = crate::store::create_private_dir(&self.inner.state_dir);
        let child = Command::new(&helper)
            .arg("--state")
            .arg(&self.inner.state_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                *lock(&self.inner.retry_at) = Some(Instant::now() + RESTART_AFTER);
                self.set_error(Some(format!("동기화 헬퍼를 못 띄웠어요: {e}")));
                return;
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return;
        };
        let gen = {
            let mut n = lock(&self.inner.gen);
            *n += 1;
            *n
        };
        *g = Some(Running { child, stdin, gen });
        drop(g);
        *lock(&self.inner.announced) = 0;
        lock(&self.inner.inflight).clear();
        let t = self.clone();
        std::thread::Builder::new()
            .name("ouro-sync-read".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) => t.handle(&v),
                        Err(_) => eprintln!("ouro-sync: {line}"),
                    }
                }
                t.reaped(gen);
            })
            .expect("동기화 리더를 띄우지 못했어요");
    }

    /// 헬퍼가 끝났다. 켜져 있으면 잠시 뒤 다시 띄운다(tick).
    fn reaped(&self, gen: u64) {
        let mut g = lock(&self.inner.child);
        if g.as_ref().is_some_and(|r| r.gen == gen) {
            if let Some(mut r) = g.take() {
                let _ = r.child.kill();
                let _ = r.child.wait();
            }
            *lock(&self.inner.retry_at) = Some(Instant::now() + RESTART_AFTER);
        }
    }

    fn stop(&self) {
        if let Some(mut r) = lock(&self.inner.child).take() {
            let _ = writeln!(r.stdin, "{}", serde_json::json!({"op": "quit"}));
            drop(r.stdin);
            // 표준입력이 닫히면 헬퍼가 스스로 끝난다. 그래도 남으면 친다.
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = r.child.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = r.child.kill();
            let _ = r.child.wait();
        }
    }

    /// 헬퍼 메시지 하나.
    fn handle(&self, v: &Value) {
        let ev = v.get("ev").and_then(Value::as_str).unwrap_or_default();
        if std::env::var_os("OURO_SYNC_TRACE").is_some() {
            let mut s = v.to_string();
            s.truncate(400);
            eprintln!("[{}] {s}", self.inner.this_name);
        }
        let id = v.get("id").cloned().unwrap_or(Value::Null);
        match ev {
            "ready" => {
                let account = v.get("account").and_then(Value::as_str).map(str::to_string);
                let error = match account.as_deref() {
                    Some("available") => None,
                    Some("noAccount") => Some("이 맥이 iCloud 에 로그인돼 있지 않아요".into()),
                    Some("restricted") => Some("이 맥에서 iCloud 를 쓸 수 없게 막혀 있어요".into()),
                    _ => Some("iCloud 계정을 확인하지 못했어요 — 잠시 뒤 다시 해 볼게요".into()),
                };
                lock(&self.inner.status).account = account;
                self.set_error(error);
                self.announce();
            }
            "need" => {
                let names: Vec<String> = v
                    .get("names")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let (records, missing) = self.provide(&names);
                self.send(&serde_json::json!({"op": "records", "id": id, "records": records, "missing": missing}));
            }
            "fetched" => {
                let records: Vec<Record> =
                    v.get("records").and_then(|r| serde_json::from_value(r.clone()).ok()).unwrap_or_default();
                let deleted: Vec<(String, String)> = v
                    .get("deleted")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|d| Some((d.get("type")?.as_str()?.to_string(), d.get("name")?.as_str()?.to_string())))
                            .collect()
                    })
                    .unwrap_or_default();
                match apply(&self.inner.store, &records, &deleted) {
                    Ok(changed) => {
                        // 적은 뒤에만 ack — ack 를 받아야 헬퍼가 «여기까지 받음» 을 저장한다.
                        self.send(&serde_json::json!({"op": "ack", "id": id}));
                        if changed {
                            (self.inner.on_change)(Change::Data);
                        }
                    }
                    Err(e) => {
                        // ack 를 안 보내면 헬퍼가 영영 기다린다 — 헬퍼를 내리고 다시 띄워 같은 변경을 다시 받는다.
                        self.set_error(Some(format!("받은 변경을 적지 못했어요: {e}")));
                        self.stop();
                        *lock(&self.inner.retry_at) = Some(Instant::now() + RESTART_AFTER);
                    }
                }
            }
            "sent" => self.on_sent(v),
            "synced" => {
                lock(&self.inner.status).last_sync = Some(now_ms());
                if v.get("reason").and_then(Value::as_str) == Some("fetch") {
                    // 첫 받기가 끝났는데 실행 맥이 없다 → 이 맥이 맡는다.
                    if let Ok(true) = claim_runner(&self.inner.store, &self.inner.this_name, true) {
                        (self.inner.on_change)(Change::Data);
                    }
                }
                self.set_error(None);
            }
            "account" => {
                let a = v.get("account").and_then(Value::as_str).unwrap_or_default();
                if a == "signOut" || a == "switchAccounts" {
                    self.turn_off_because("iCloud 계정이 바뀌어 동기화를 껐어요");
                }
            }
            "zone_gone" => {
                if v.get("reason").and_then(Value::as_str) == Some("encryptedDataReset") {
                    // 암호 키가 초기화됨 — 전부 다시 올린다(헬퍼가 존을 다시 만든다).
                    let _ = forget_server(&self.inner.store);
                    let _ = enqueue_all(&self.inner.store.conn());
                    *lock(&self.inner.announced) = 0;
                } else {
                    self.turn_off_because("iCloud 에서 Ouro 데이터가 지워져 동기화를 껐어요");
                }
            }
            "error" => self.set_error(v.get("message").and_then(Value::as_str).map(|m| format!("동기화 오류: {m}"))),
            "log" => eprintln!("ouro-sync: {}", v.get("message").and_then(Value::as_str).unwrap_or_default()),
            _ => {}
        }
    }

    fn provide(&self, names: &[String]) -> (Vec<Record>, Vec<String>) {
        let conn = self.inner.store.conn();
        let (mut records, mut missing) = (vec![], vec![]);
        for n in names {
            let found = locate(&conn, n).ok().flatten().and_then(|(ty, key)| {
                let rec = build(&conn, ty, n).ok().flatten()?;
                let seq: Option<i64> =
                    conn.query_row("SELECT seq FROM sync_outbox WHERE key = ?1", [&key], |r| r.get(0)).optional().ok().flatten();
                lock(&self.inner.inflight).insert(n.clone(), (key, seq.unwrap_or(i64::MAX)));
                Some(rec)
            });
            match found {
                Some(r) => records.push(r),
                None => missing.push(n.clone()),
            }
        }
        (records, missing)
    }

    fn on_sent(&self, v: &Value) {
        let conn = self.inner.store.conn();
        let mut retry = false;
        for s in v.get("saved").and_then(Value::as_array).into_iter().flatten() {
            let (Some(name), Some(sys)) = (s.get("name").and_then(Value::as_str), s.get("system").and_then(Value::as_str)) else {
                continue;
            };
            let _ = conn.execute(
                "INSERT INTO sync_meta (name, system) VALUES (?1, ?2) ON CONFLICT (name) DO UPDATE SET system = excluded.system",
                params![name, sys],
            );
            if let Some((key, seq)) = lock(&self.inner.inflight).remove(name) {
                let _ = conn.execute("DELETE FROM sync_outbox WHERE key = ?1 AND seq <= ?2", params![key, seq]);
            }
        }
        for n in v.get("removed").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            let _ = conn.execute("DELETE FROM sync_outbox WHERE key = ?1", [format!("del:{n}")]);
            let _ = conn.execute("DELETE FROM sync_meta WHERE name = ?1", [n]);
        }
        let mut error = None;
        for f in v.get("failed").and_then(Value::as_array).into_iter().flatten() {
            let name = f.get("name").and_then(Value::as_str).unwrap_or_default();
            match f.get("code").and_then(Value::as_str).unwrap_or_default() {
                "unknownItem" => {
                    // 서버에 없는 레코드 — 옛 시스템 칸으로 «고치기» 를 하면 계속 실패한다. 새로 올린다. 삭제였다면 이미 없는 것.
                    let _ = conn.execute("DELETE FROM sync_meta WHERE name = ?1", [name]);
                    let _ = conn.execute("DELETE FROM sync_outbox WHERE key = ?1", [format!("del:{name}")]);
                }
                "quotaExceeded" => error = Some("iCloud 저장 공간이 꽉 찼어요".to_string()),
                "serverRecordChanged" => {} // 서버 판은 `fetched` 로 이미 합쳤다. 내 쪽이 이겼으면 보낼 목록에 다시 들어가 있다.
                _ => {}
            }
            lock(&self.inner.inflight).remove(name);
            retry = true;
        }
        drop(conn);
        if retry {
            // 남은 보낼 목록을 처음부터 다시 알린다 — 실패한 이름이 헬퍼의 목록에서 빠졌다.
            *lock(&self.inner.announced) = 0;
        }
        if error.is_some() {
            self.set_error(error);
        }
        self.refresh_status();
    }

    fn turn_off_because(&self, why: &str) {
        let _ = set_on(&self.inner.store, false);
        let _ = forget_server(&self.inner.store);
        let _ = std::fs::remove_file(self.inner.state_dir.join("engine.json"));
        self.set_error(Some(why.into()));
        let me = self.clone();
        // 리더 스레드 안에서 부르므로 stop 은 따로(stop 이 리더가 끝나길 기다리지 않지만, 자기 표준입력을 닫는 일은 밖에서).
        std::thread::spawn(move || me.stop());
        (self.inner.on_change)(Change::Data);
    }

    fn set_error(&self, e: Option<String>) {
        lock(&self.inner.status).error = e;
        self.refresh_status();
    }

    fn refresh_status(&self) {
        let conn = self.inner.store.conn();
        let on = flag(&conn, "on").unwrap_or(0) == 1;
        let pending: i64 = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0)).unwrap_or(0);
        let runner = setting(&conn, "sync_runner").ok().flatten().filter(|r| !r.is_empty());
        let me = device_id(&conn).unwrap_or_default();
        let runner_name = setting(&conn, "sync_runner_name").ok().flatten();
        drop(conn);
        let changed = {
            let mut s = lock(&self.inner.status);
            let before = s.clone();
            s.available = self.inner.helper.is_some();
            s.on = on;
            s.pending = pending;
            s.runner_is_me = runner.as_deref() == Some(me.as_str());
            s.runner_name = runner.and(runner_name);
            s.this_name = self.inner.this_name.clone();
            *s != before
        };
        if changed {
            (self.inner.on_change)(Change::Status);
        }
    }

    pub(crate) fn status(&self) -> SyncStatus {
        self.refresh_status();
        lock(&self.inner.status).clone()
    }

    /// 사람이 켜고 끈다.
    pub(crate) fn set_enabled(&self, on: bool) -> Result<SyncStatus, String> {
        if on && self.inner.helper.is_none() {
            return Err("이 앱에는 동기화 헬퍼가 없어요".into());
        }
        set_on(&self.inner.store, on)?;
        if on {
            lock(&self.inner.status).error = None;
            *lock(&self.inner.retry_at) = None;
            self.spawn();
        } else {
            self.stop();
            lock(&self.inner.status).error = None;
        }
        (self.inner.on_change)(Change::Data);
        Ok(self.status())
    }

    /// 이 맥을 실행 맥으로(사람이 누름).
    pub(crate) fn make_runner(&self) -> Result<SyncStatus, String> {
        claim_runner(&self.inner.store, &self.inner.this_name, false)?;
        (self.inner.on_change)(Change::Data);
        Ok(self.status())
    }

    /// 지금 받기(팝오버를 열 때). 켜져 있고 헬퍼가 떠 있을 때만.
    pub(crate) fn fetch_now(&self) {
        self.send(&serde_json::json!({"op": "fetch"}));
    }

    pub(crate) fn shutdown(&self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EventInput;

    fn event(title: &str, at: i64) -> EventInput {
        EventInput {
            title: title.into(),
            notes: "메모".into(),
            all_day: false,
            start_at: Some(at),
            end_at: Some(at + 3_600_000),
            start_date: None,
            end_date: None,
            alert_min: None,
            private: false,
        }
    }

    fn uid_of(s: &Store, table: &str, id: i64) -> String {
        s.conn().query_row(&format!("SELECT uid FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).unwrap()
    }

    /// A 의 보낼 목록을 레코드로 꺼낸다(헬퍼 대신).
    fn drain(s: &Store) -> (Vec<Record>, Vec<(String, String)>) {
        let conn = s.conn();
        let rows = outbox_since(&conn, 0).unwrap();
        let mut recs = vec![];
        let mut dels = vec![];
        for p in rows {
            if p.save {
                recs.push(build(&conn, &p.ty, &p.name).unwrap().unwrap());
            } else {
                dels.push((p.ty, p.name));
            }
        }
        conn.execute("DELETE FROM sync_outbox", []).unwrap();
        (recs, dels)
    }

    #[test]
    fn nothing_is_tracked_until_sync_is_on() {
        let s = Store::open_in_memory();
        s.create_event(&event("치과", 1_800_000_000_000)).unwrap();
        assert!(outbox_since(&s.conn(), 0).unwrap().is_empty());
        set_on(&s, true).unwrap();
        let rows = outbox_since(&s.conn(), 0).unwrap();
        assert_eq!(rows.len(), 1, "켤 때 가진 것을 전부 싣는다");
        assert_eq!(rows[0].ty, "Item");
    }

    #[test]
    fn edits_and_deletes_are_tracked_by_uid() {
        let s = Store::open_in_memory();
        set_on(&s, true).unwrap();
        let e = s.create_event(&event("치과", 1_800_000_000_000)).unwrap();
        let uid = uid_of(&s, "items", e.id);
        let rows = outbox_since(&s.conn(), 0).unwrap();
        assert_eq!(rows.iter().filter(|p| p.save && p.name == uid).count(), 1, "INSERT + uid 채우기 = 한 줄");
        s.conn().execute("DELETE FROM items WHERE id = ?1", [e.id]).unwrap();
        let rows = outbox_since(&s.conn(), 0).unwrap();
        assert_eq!(rows, vec![Pending { key: format!("del:{uid}"), ty: "Item".into(), name: uid, save: false, seq: rows[0].seq }]);
        // rowid 가 재사용돼도 삭제 줄이 덮이지 않는다.
        let e2 = s.create_event(&event("새 일정", 1_800_000_000_000)).unwrap();
        assert_eq!(e2.id, e.id, "SQLite 는 마지막 rowid 를 다시 쓴다");
        let rows = outbox_since(&s.conn(), 0).unwrap();
        assert_eq!(rows.iter().filter(|p| !p.save).count(), 1);
        assert_eq!(rows.iter().filter(|p| p.save).count(), 1);
    }

    #[test]
    fn records_hide_text_in_secret_fields() {
        let s = Store::open_in_memory();
        set_on(&s, true).unwrap();
        let e = s.create_event(&event("치과", 1_800_000_000_000)).unwrap();
        let uid = uid_of(&s, "items", e.id);
        let r = build(&s.conn(), "Item", &uid).unwrap().unwrap();
        assert_eq!(r.secret.get("title"), Some(&Value::from("치과")));
        assert_eq!(r.secret.get("notes"), Some(&Value::from("메모")));
        assert!(r.fields.get("title").is_none());
        assert_eq!(r.fields.get("start_at"), Some(&Value::from(1_800_000_000_000i64)));
        assert_eq!(r.fields.get("prompt"), None, "부탁 글은 비밀 칸");
        assert_eq!(r.secret.get("prompt"), Some(&Value::Null), "일정이면 부탁 칸은 비운다");
    }

    #[test]
    fn two_devices_converge_and_received_changes_are_not_echoed() {
        let a = Store::open_in_memory();
        let b = Store::open_in_memory();
        set_on(&a, true).unwrap();
        set_on(&b, true).unwrap();
        let e = a.create_event(&event("치과", 1_800_000_000_000)).unwrap();
        let (recs, dels) = drain(&a);
        assert!(apply(&b, &recs, &dels).unwrap());
        let got = b.list_events(0, i64::MAX).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "치과");
        assert!(outbox_since(&b.conn(), 0).unwrap().is_empty(), "받은 걸 도로 보내지 않는다");
        // B 에서 고치면 A 로.
        let mut input = event("치과 (옮김)", 1_800_000_000_000 + 3_600_000);
        input.notes = "".into();
        std::thread::sleep(Duration::from_millis(2));
        b.update_event(got[0].id, &input).unwrap();
        let (recs, dels) = drain(&b);
        apply(&a, &recs, &dels).unwrap();
        assert_eq!(a.get_event(e.id).unwrap().unwrap().title, "치과 (옮김)");
        // A 에서 지우면(휴지통) B 도.
        a.delete_event(e.id).unwrap();
        let (recs, dels) = drain(&a);
        apply(&b, &recs, &dels).unwrap();
        assert!(b.list_events(0, i64::MAX).unwrap().is_empty());
    }

    #[test]
    fn older_remote_edit_loses_and_local_is_resent() {
        let a = Store::open_in_memory();
        let b = Store::open_in_memory();
        set_on(&a, true).unwrap();
        set_on(&b, true).unwrap();
        let e = a.create_event(&event("원래", 1_800_000_000_000)).unwrap();
        let (recs, _) = drain(&a);
        apply(&b, &recs, &[]).unwrap();
        let old = recs[0].clone();
        std::thread::sleep(Duration::from_millis(2));
        a.update_event(e.id, &event("새것", 1_800_000_000_000)).unwrap();
        drain(&a);
        // 옛 판(서버가 늦게 준)이 와도 A 의 새것이 남고, A 는 다시 보낸다.
        apply(&a, &[old], &[]).unwrap();
        assert_eq!(a.get_event(e.id).unwrap().unwrap().title, "새것");
        assert_eq!(outbox_since(&a.conn(), 0).unwrap().len(), 1);
    }

    #[test]
    fn run_before_its_errand_is_parked_then_resolved() {
        let a = Store::open_in_memory();
        let b = Store::open_in_memory();
        set_on(&a, true).unwrap();
        set_on(&b, true).unwrap();
        let er = a
            .create_errand(&crate::errands::ErrandInput { prompt: "요약해 줘".into(), start_at: now_ms() + 3_600_000, ..Default::default() })
            .unwrap();
        let run = a.begin_run(er.id, "요약해 줘", 0, None).unwrap();
        let (recs, _) = drain(&a);
        let (items, runs): (Vec<Record>, Vec<Record>) = recs.into_iter().partition(|r| r.ty == "Item");
        apply(&b, &runs, &[]).unwrap();
        let parked: i64 = b.conn().query_row("SELECT COUNT(*) FROM sync_inbox", [], |r| r.get(0)).unwrap();
        assert_eq!(parked, 1, "부탁이 없으니 실행은 세워 둔다");
        apply(&b, &items, &[]).unwrap();
        let parked: i64 = b.conn().query_row("SELECT COUNT(*) FROM sync_inbox", [], |r| r.get(0)).unwrap();
        assert_eq!(parked, 0);
        let run_uid = uid_of(&a, "runs", run);
        let (item_id, device): (i64, String) =
            b.conn().query_row("SELECT item_id, device FROM runs WHERE uid = ?1", [&run_uid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(item_id, b.conn().query_row("SELECT id FROM items WHERE uid = ?1", [uid_of(&a, "items", er.id)], |r| r.get::<_, i64>(0)).unwrap());
        assert_eq!(device, device_id(&a.conn()).unwrap(), "돌린 기기는 A");
        let e = b.get_errand(item_id).unwrap().unwrap();
        assert_eq!(e.prompt, "요약해 줘");
        assert!(e.approved);
    }

    #[test]
    fn run_merge_keeps_finished_and_read() {
        let mut local = Map::new();
        local.insert("status".into(), "running".into());
        local.insert("finished_at".into(), Value::Null);
        local.insert("read_at".into(), Value::Null);
        let mut remote = Map::new();
        remote.insert("status".into(), "done".into());
        remote.insert("finished_at".into(), 5.into());
        remote.insert("read_at".into(), Value::Null);
        assert_eq!(merge("Run", &local, &remote)["status"], "done");
        // 거꾸로: 내가 끝냈고 서버가 도는 중 → 내 판, 서버가 읽음이면 읽음.
        let mut l2 = remote.clone();
        l2.insert("read_at".into(), Value::Null);
        let mut r2 = local.clone();
        r2.insert("read_at".into(), 9.into());
        let m = merge("Run", &l2, &r2);
        assert_eq!(m["status"], "done");
        assert_eq!(m["read_at"], 9);
    }

    #[test]
    fn proposal_decision_is_not_undone() {
        let mut decided = Map::new();
        decided.insert("status".into(), "approved".into());
        decided.insert("decided_at".into(), 10.into());
        let mut pending = Map::new();
        pending.insert("status".into(), "pending".into());
        assert_eq!(merge("Proposal", &decided, &pending)["status"], "approved");
        let mut rejected_later = Map::new();
        rejected_later.insert("status".into(), "rejected".into());
        rejected_later.insert("decided_at".into(), 20.into());
        assert_eq!(merge("Proposal", &decided, &rejected_later)["status"], "approved", "먼저 결정한 쪽");
    }

    #[test]
    fn remote_delete_keeps_errand_that_ran() {
        let a = Store::open_in_memory();
        set_on(&a, true).unwrap();
        let er = a
            .create_errand(&crate::errands::ErrandInput { prompt: "요약".into(), start_at: now_ms() + 3_600_000, ..Default::default() })
            .unwrap();
        a.begin_run(er.id, "요약", 0, None).unwrap();
        drain(&a);
        let uid = uid_of(&a, "items", er.id);
        apply(&a, &[], &[("Item".into(), uid.clone())]).unwrap();
        assert!(a.get_errand(er.id).unwrap().is_some(), "돈 부탁은 남는다");
        assert!(outbox_since(&a.conn(), 0).unwrap().iter().any(|p| p.name == uid && p.save), "서버에 다시 올린다");
    }

    #[test]
    fn runner_gates_scheduled_runs_only_when_sync_is_on() {
        let s = Store::open_in_memory();
        assert!(is_runner(&s.conn()).unwrap(), "혼자 쓰면 언제나 돌린다");
        set_on(&s, true).unwrap();
        assert!(!is_runner(&s.conn()).unwrap(), "켰는데 실행 맥이 없으면 아무도 안 돌린다");
        let er = s
            .create_errand(&crate::errands::ErrandInput { prompt: "요약".into(), start_at: now_ms() - 1000, ..Default::default() })
            .unwrap();
        let skip = |_: &crate::errands::Due, _: i64| None;
        assert!(s.claim_run(er.id, crate::errands::Claim::Scheduled, skip).unwrap().is_none());
        assert!(claim_runner(&s, "이 맥", true).unwrap());
        assert!(!claim_runner(&s, "이 맥", true).unwrap(), "이미 있으면 안 뺏는다");
        assert!(is_runner(&s.conn()).unwrap());
        assert!(s.claim_run(er.id, crate::errands::Claim::Scheduled, skip).unwrap().is_some());
        // 다른 맥이 실행 맥을 가져가면 이 맥은 멈춘다.
        let mut cfg = Map::new();
        cfg.insert("device".into(), "other-mac".into());
        cfg.insert("name".into(), "다른 맥".into());
        cfg.insert("at".into(), (now_ms() + 10).into());
        let rec = split(CONFIG_TYPE, RUNNER, cfg, None);
        apply(&s, &[rec], &[]).unwrap();
        assert!(!is_runner(&s.conn()).unwrap());
    }

    #[test]
    fn orphan_runs_of_other_devices_are_left_alone() {
        let s = Store::open_in_memory();
        set_on(&s, true).unwrap();
        let er = s
            .create_errand(&crate::errands::ErrandInput { prompt: "요약".into(), start_at: now_ms() + 3_600_000, ..Default::default() })
            .unwrap();
        let mine = s.begin_run(er.id, "요약", 0, None).unwrap();
        let theirs = s.begin_run(er.id, "요약", 0, None).unwrap();
        s.conn().execute("UPDATE runs SET device = 'other-mac' WHERE id = ?1", [theirs]).unwrap();
        assert_eq!(s.fail_orphan_runs().unwrap(), 1);
        let st = |id: i64| -> String { s.conn().query_row("SELECT status FROM runs WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
        assert_eq!(st(mine), "failed");
        assert_eq!(st(theirs), "running", "다른 맥이 돌리는 중인 건 닫지 않는다");
        assert!(!s.any_running().unwrap(), "업데이트 설치를 막는 건 이 맥의 실행만");
    }

    /// 진짜 CloudKit(개발 환경) 왕복. 서명된 헬퍼가 필요하다:
    /// `OURO_SYNC_HELPER=$PWD/sync/build/Build/Products/Debug/OuroSync.app/Contents/MacOS/OuroSync cargo test -- --ignored cloudkit`
    #[test]
    #[ignore]
    fn cloudkit_round_trip_between_two_homes() {
        let helper = find_helper().expect("OURO_SYNC_HELPER 를 주세요");
        let (da, db) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let a = Arc::new(Store::open(da.path()).unwrap());
        let b = Arc::new(Store::open(db.path()).unwrap());
        let noop: OnChange = Arc::new(|_| {});
        let sa = Syncer::start(a.clone(), da.path(), Some(helper.clone()), "맥 A".into(), noop.clone());
        sa.set_enabled(true).unwrap();
        let wait = |what: &str, f: &dyn Fn() -> bool| {
            let end = Instant::now() + Duration::from_secs(120);
            while Instant::now() < end {
                if f() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            panic!("{what} 를 기다리다 시간 초과");
        };
        let title = format!("동기화 확인 {}", now_ms());
        let e = a.create_event(&event(&title, 1_900_000_000_000)).unwrap();
        let uid = uid_of(&a, "items", e.id);
        // 개발 존엔 지난 테스트의 «실행 맥» 이 남아 있을 수 있다 — 사람이 «이 맥으로» 를 누르는 것과 같은 길로 가져온다.
        wait("A 의 첫 받기", &|| sa.status().last_sync.is_some());
        sa.make_runner().unwrap();
        assert!(is_runner(&a.conn()).unwrap());
        wait("A 의 보낼 목록 비우기", &|| {
            a.conn().query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get::<_, i64>(0)).unwrap() == 0
        });
        let sb = Syncer::start(b.clone(), db.path(), Some(helper), "맥 B".into(), noop);
        sb.set_enabled(true).unwrap();
        wait("B 가 받기", &|| {
            b.conn().query_row("SELECT COUNT(*) FROM items WHERE uid = ?1", [&uid], |r| r.get::<_, i64>(0)).unwrap() == 1
        });
        let got: String = b.conn().query_row("SELECT title FROM items WHERE uid = ?1", [&uid], |r| r.get(0)).unwrap();
        assert_eq!(got, title);
        wait("B 가 실행 맥을 앎", &|| sb.status().runner_name.as_deref() == Some("맥 A"));
        assert!(!is_runner(&b.conn()).unwrap(), "실행 맥은 하나");
        // B 에서 지우면(휴지통 비우기 = 진짜 삭제) 서버에서도 사라진다. 🔴 같은 맥의 A 는 이걸 «이어 받기» 로 못 받는다 —
        // CloudKit 은 변경을 만든 **기기**에 그 변경을 다시 주지 않고, 같은 맥의 두 프로세스는 같은 기기다(개발 10 실측,
        // developer.apple.com/forums/thread/774014). 그래서 새 엔진(C, 처음 받기)으로 서버 상태를 본다. 기기 사이의 이어 받기는 iOS 시뮬레이터로.
        let bid: i64 = b.conn().query_row("SELECT id FROM items WHERE uid = ?1", [&uid], |r| r.get(0)).unwrap();
        b.conn().execute("DELETE FROM items WHERE id = ?1", [bid]).unwrap();
        wait("B 의 삭제 보내기", &|| {
            b.conn().query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get::<_, i64>(0)).unwrap() == 0
        });
        let dc = tempfile::tempdir().unwrap();
        let c = Arc::new(Store::open(dc.path()).unwrap());
        let sc = Syncer::start(c.clone(), dc.path(), find_helper(), "맥 C".into(), Arc::new(|_| {}));
        sc.set_enabled(true).unwrap();
        wait("C 의 첫 받기", &|| sc.status().last_sync.is_some() && sc.status().runner_name.is_some());
        let n: i64 = c.conn().query_row("SELECT COUNT(*) FROM items WHERE uid = ?1", [&uid], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "서버에서 지워졌다");
        assert_eq!(sc.status().runner_name.as_deref(), Some("맥 A"));
        sc.shutdown();
        sa.shutdown();
        sb.shutdown();
    }
}
