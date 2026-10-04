// MCP 사이드카가 묻는 문 — Unix 소켓 `~/.ouro/ouro.sock` (PLAN §6).
//
// 사이드카(`ouro-mcp`)는 DB 를 열지 않는다. 도구가 불리면 여기로 한 줄짜리 JSON 을 보내고 한 줄짜리 답을 받는다.
// 판단(검사·시각 계산·비공개 거르기)은 전부 이쪽에 있고 사이드카는 옮겨 주기만 한다 — 그래서 테스트도 여기서 한다(`handle`).
//
// 설계 결정:
//   · **승인 문이 없다.** 소켓이 받는 일은 읽기(agenda·free_time), 제안 넣기(propose·propose_errand), 제안 상태 보기(proposal·errand_proposal) 여섯뿐이다.
//     제안을 일정·부탁으로 만드는 건 팝오버의 `approve_proposal`·`approve_errand_proposal` 커맨드 하나씩 — AI 가 자기 제안을 스스로 받을 길이 원리상 없다.
//     (CLAUDE.md «AI 가 만든 부탁은 사람 승인 전엔 안 돈다» — 부탁 제안은 그 규칙 그대로, 일정 제안도 같은 모양.)
//   · **부탁 제안의 답엔 내용이 안 돌아간다.** 승인된 뒤엔 상태(`run_status`)와 실행 번호만 — 답 본문은 안 준다. 제안 번호는 순서대로라 다른 AI 가
//     짐작해 물을 수 있고, 사람이 고쳐서 받은 글·돌아온 답이 그 문으로 새면 안 된다(개발 4 코덱스가 일정에서 짚은 것과 같은 이유).
//   · **시각은 로컬 벽시계 글자**(`2026-10-02T15:00`)로 주고받는다. AI 가 UTC ms 를 셈하게 두면 시간대·서머타임에서 틀린다.
//     글자 → 순간 변환은 여기서 결정적으로 한다(CLAUDE.md «날짜 계산은 LLM 이 아니라 결정적 파서가»). 없는 시각(서머타임 틈)은 거부.
//     답마다 `now`(요일 포함)를 싣는다 — «금요일» 을 날짜로 바꾸는 건 AI 지만, 오늘이 무슨 요일인지는 이 답이 정본이다.
//   · 하트비트 파일은 두지 않는다(PLAN §12 의 «하트비트» 를 소켓이 대신한다). Kura 는 파일로 주고받아서 «앱이 살아 있나» 를 따로 적어야 했지만,
//     소켓은 연결이 되면 살아 있는 것이고 안 되면(없음·거부) 꺼진 것이다. 제안은 DB 에 남으니 화면이 잠깐 죽어 있어도 사라지지 않는다.
//   · 반대 방향(«AI 가 붙어 있나») 은 사이드카가 15초마다 `hello` 를 보내 알린다(`Presence`, 메모리에만). 팝오버 아래 «◯ Claude Code 연결됨».
//     사이드카 프로세스마다 `instance`(pid)로 따로 센다 — Claude Code 세션 둘이 같은 이름으로 붙어도 하나가 나갈 때 다른 쪽이 안 지워지게.
//   · 권한: 소켓은 0700 폴더 안의 0600 파일 — 같은 사용자만 연결한다.
//   · 상한(개발 9, 코덱스 개발 5·6 P2): 동시 연결 32, 요청 한 줄은 연결부터 10초 안에 다 와야 한다. 읽기 시간 제한만 두면
//     한 바이트씩 4초마다 흘리는 연결이 스레드를 며칠 붙든다 — 같은 사용자의 프로세스만 붙지만, 고장 난 클라이언트가 앱을 굶기지 않게.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Weekday};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::errands::{ErrandInput, ErrandProposal, ErrandProposalInput, TOOL_CHOICES};
use crate::store::{self, Event, EventInput, Proposal, Store};

/// 동시에 붙들 수 있는 연결 수. 넘으면 받자마자 닫는다(사이드카는 «앱이 바빠요» 로 읽고 다시 묻는다).
const CONN_MAX: usize = 32;
/// 요청 한 줄이 다 와야 하는 시한(연결부터).
const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

/// 요청 한 줄 상한. 제목·메모 상한(store.rs)보다 넉넉하되 거대한 글은 읽지도 않는다.
const REQUEST_MAX: u64 = 64 * 1024;
/// 한 번에 읽을 수 있는 기간 — AI 가 «올해 전부» 를 긁어 가지 않게.
const AGENDA_MAX_DAYS: u64 = 62;
const FREE_MAX_DAYS: u64 = 31;
const FREE_MAX_SLOTS: usize = 50;
const MIN: i64 = 60 * 1000;

pub(crate) fn socket_path(dir: &Path) -> PathBuf {
    dir.join("ouro.sock")
}

#[derive(Debug, Deserialize)]
struct Request {
    #[serde(default)]
    client: String,
    /// 사이드카 프로세스 하나를 가리키는 값(pid). 연결 표시를 프로세스별로 센다.
    #[serde(default)]
    instance: String,
    op: String,
    #[serde(default)]
    args: Value,
}

/// 소켓에서 생긴 일 — 앱이 화면에 알린다. 테스트에선 아무것도 안 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// 새 제안 → 팝오버를 띄우고 카드를 갱신.
    Proposal,
    /// 붙은 AI 목록이 바뀜(새로 붙음·나감) → 연결 줄만 갱신.
    Presence,
}
pub(crate) type OnChange = Arc<dyn Fn(Change) + Send + Sync>;

/// 이만큼 소식이 없으면 끊긴 것으로 본다 — 사이드카는 15초마다 hello 를 보내니 세 번 놓친 셈.
const ALIVE: Duration = Duration::from_secs(45);
/// 동시에 셀 사이드카 수·instance 길이 상한 — 같은 사용자의 아무 프로세스나 소켓에 쓸 수 있으니 맵이 끝없이 크지 않게(코덱스 개발 4 보완).
const PRESENCE_MAX: usize = 64;
const INSTANCE_MAX: usize = 32;

/// 지금 붙어 있는 MCP 사이드카들(메모리에만 — 앱을 껐다 켜면 다음 hello 에 다시 채워진다).
/// 시간은 **단조 시계**(`Instant`) — 시스템 시각을 뒤로 돌려도 끊긴 세션이 «연결됨» 으로 남지 않게(코덱스 개발 4 보완).
#[derive(Default)]
pub(crate) struct Presence {
    inner: std::sync::Mutex<PresenceMap>,
}

#[derive(Default)]
struct PresenceMap {
    /// instance → (클라이언트 이름, 마지막 소식)
    seen: std::collections::HashMap<String, (String, Instant)>,
    /// bye 를 보낸 instance — 연결마다 스레드라 bye 가 그 앞에 보낸 hello 보다 먼저 처리될 수 있다.
    /// 늦게 온 hello 가 나간 세션을 되살리지 않게 ALIVE 동안 기억한다(코덱스 개발 4 보완).
    left: std::collections::HashMap<String, Instant>,
}

impl PresenceMap {
    fn prune(&mut self, now: Instant) {
        self.seen.retain(|_, (_, t)| now.saturating_duration_since(*t) <= ALIVE);
        self.left.retain(|_, t| now.saturating_duration_since(*t) <= ALIVE);
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.seen.values().map(|(c, _)| c.clone()).collect();
        v.sort();
        v.dedup();
        v
    }
}

impl Presence {
    fn lock(&self) -> std::sync::MutexGuard<'_, PresenceMap> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 소식 하나를 적는다. 붙은 목록이 바뀌었으면 true(화면을 갱신할 때).
    fn touch(&self, instance: &str, client: &str, now: Instant) -> bool {
        let key: String = instance.chars().take(INSTANCE_MAX).collect();
        let mut m = self.lock();
        m.prune(now);
        let before = m.names();
        if m.left.contains_key(&key) || (!m.seen.contains_key(&key) && m.seen.len() >= PRESENCE_MAX) {
            return false;
        }
        m.seen.insert(key, (client.to_string(), now));
        m.names() != before
    }

    fn leave(&self, instance: &str, now: Instant) -> bool {
        let key: String = instance.chars().take(INSTANCE_MAX).collect();
        let mut m = self.lock();
        m.prune(now);
        let before = m.names();
        m.seen.remove(&key);
        if m.left.len() < PRESENCE_MAX {
            m.left.insert(key, now);
        }
        m.names() != before
    }

    /// 붙어 있는 클라이언트 이름(중복 없이, 정렬).
    pub(crate) fn clients(&self, now: Instant) -> Vec<String> {
        let mut m = self.lock();
        m.prune(now);
        m.names()
    }
}

/// 소켓을 열고 받는 스레드를 띄운다. 다른 Ouro 가 이미 소켓을 쥐고 있으면 Err(그쪽을 빼앗지 않는다).
pub(crate) fn serve(store: Arc<Store>, presence: Arc<Presence>, dir: &Path, on_change: OnChange) -> Result<(), String> {
    let path = socket_path(dir);
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err("다른 Ouro 가 이미 떠 있어요".into());
        }
        // 지난번 앱이 죽으며 남긴 소켓 파일 — 연결을 안 받으니 지우고 새로 연다.
        std::fs::remove_file(&path).map_err(|e| format!("옛 소켓 지우기 실패: {e}"))?;
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("소켓 열기 실패: {e}"))?;
    store::restrict_file(&path)?;
    std::thread::Builder::new()
        .name("ouro-mcp-socket".into())
        .spawn(move || {
            let active = Arc::new(AtomicUsize::new(0));
            for conn in listener.incoming().flatten() {
                if active.fetch_add(1, Ordering::SeqCst) >= CONN_MAX {
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(conn);
                    continue;
                }
                let slot = Slot(active.clone());
                let store = store.clone();
                let presence = presence.clone();
                let on_change = on_change.clone();
                // 연결 하나 = 요청 하나(짧다). 느린 연결 하나가 다른 걸 막지 않게 스레드로.
                let _ = std::thread::Builder::new()
                    .name("ouro-mcp-conn".into())
                    .spawn(move || {
                        let _slot = slot;
                        serve_one(conn, &store, &presence, &on_change)
                    });
            }
        })
        .map_err(|e| format!("소켓 스레드 실패: {e}"))?;
    Ok(())
}

/// 연결 자리 하나 — 스레드가 끝나면(패닉 포함) 돌려준다.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 한 줄을 `deadline` 안에 읽는다(최대 `max` 바이트). 시한을 넘기거나 끊기면 None.
fn read_line_by(conn: &mut UnixStream, max: u64, deadline: Instant) -> Option<String> {
    let mut buf: Vec<u8> = vec![];
    let mut chunk = [0u8; 4096];
    while !buf.contains(&b'\n') && (buf.len() as u64) < max {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
        conn.set_read_timeout(Some(left)).ok()?;
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n.min((max as usize).saturating_sub(buf.len()))]),
            Err(_) => return None,
        }
    }
    let end = buf.iter().position(|&b| b == b'\n').unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

fn serve_one(mut conn: UnixStream, store: &Store, presence: &Presence, on_change: &OnChange) {
    let _ = conn.set_write_timeout(Some(Duration::from_secs(5)));
    let Some(line) = read_line_by(&mut conn, REQUEST_MAX, Instant::now() + REQUEST_DEADLINE) else { return };
    let (reply, proposed) = match serde_json::from_str::<Request>(&line) {
        Ok(req) => {
            // 어떤 요청이든 «붙어 있다» 는 소식이다. instance 가 없는 옛 사이드카는 이름으로 센다.
            if !req.instance.is_empty() || !req.client.is_empty() {
                let key = if req.instance.is_empty() { &req.client } else { &req.instance };
                let now = Instant::now();
                let changed = if req.op == "bye" {
                    presence.leave(key, now)
                } else {
                    presence.touch(key, &clean_client(&req.client), now)
                };
                if changed {
                    on_change(Change::Presence);
                }
            }
            let proposed = matches!(req.op.as_str(), "propose" | "propose_errand");
            match handle(store, &req, Local::now()) {
                Ok(data) => (json!({ "ok": true, "data": data }), proposed),
                Err(e) => (json!({ "ok": false, "error": e }), false),
            }
        }
        Err(_) => (json!({ "ok": false, "error": "요청 모양이 이상해요" }), false),
    };
    let mut w = conn;
    let _ = writeln!(w, "{reply}");
    if proposed {
        on_change(Change::Proposal);
    }
}

/// 클라이언트가 밝힌 이름을 보여 줄 모양으로(제어 문자 빼고 60자) — store 의 제안 이름과 같은 규칙.
fn clean_client(c: &str) -> String {
    match c.trim() {
        "" => "unknown".into(),
        c => c.chars().filter(|c| !c.is_control()).take(60).collect(),
    }
}

/// 요청 하나 → 답(순수에 가깝다 — DB 와 «지금» 만 본다).
/// `now` 는 오프셋을 가진 순간이다 — 가을 서머타임의 두 번째 01:30 을 벽시계 글자로만 들고 있으면 첫 번째로 되돌아간다(코덱스 개발 4).
fn handle(store: &Store, req: &Request, now_at: DateTime<Local>) -> Result<Value, String> {
    let now = now_at.naive_local();
    let body = match req.op.as_str() {
        "agenda" => agenda(store, &req.args, now)?,
        "free_time" => free_time(store, &req.args, now, now_at.timestamp_millis())?,
        "propose" => {
            let input = propose_input(&req.args)?;
            let p = store.add_proposal(&req.client, &input)?;
            proposal_json(store, &p)?
        }
        "proposal" => {
            let id = req.args.get("id").and_then(Value::as_i64).ok_or("id 가 필요해요")?;
            let p = store.get_proposal(id)?.ok_or("없는 제안이에요")?;
            proposal_status_json(store, &p)?
        }
        "propose_errand" => {
            let p = store.add_errand_proposal(&req.client, &errand_proposal_input(&req.args)?)?;
            errand_proposal_json(store, &p)?
        }
        "errand_proposal" => {
            let id = req.args.get("id").and_then(Value::as_i64).ok_or("id 가 필요해요")?;
            let p = store.get_errand_proposal(id)?.ok_or("없는 제안이에요")?;
            errand_proposal_json(store, &p)?
        }
        // 연결 표시용 — 소식은 serve_one 이 이미 적었다.
        "hello" | "bye" => json!({}),
        // 🔴 승인·거절·수정·삭제는 여기 없다 — 위 머리 주석.
        other => return Err(format!("모르는 요청이에요: {other}")),
    };
    let mut body = body;
    body["now"] = json!(format!("{} ({})", now.format("%Y-%m-%dT%H:%M"), weekday(now.date())));
    body["timezone"] = json!(iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()));
    Ok(body)
}

// ── 글자 ↔ 시각 ──────────────────────────────────────────────

fn weekday(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn parse_day(s: &str) -> Result<NaiveDate, String> {
    let s = s.trim();
    if s.len() == 10 {
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(d);
        }
    }
    Err(format!("날짜는 YYYY-MM-DD 로 주세요: {s}"))
}

/// `2026-10-02T15:00`(초는 있어도 된다) → 로컬 벽시계. 시간대 표시(Z, +09:00)는 받지 않는다 — 두 규칙이 섞이면 틀린다.
fn parse_local(s: &str) -> Result<NaiveDateTime, String> {
    let s = s.trim();
    for f in ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, f) {
            return Ok(t);
        }
    }
    Err(format!("시각은 로컬 YYYY-MM-DDTHH:MM 으로 주세요(Z·+09:00 없이): {s}"))
}

/// 로컬 벽시계 → UTC ms. 서머타임으로 건너뛰는 시각은 거부한다(조용히 한 시간 밀려 저장되지 않게, 개발 2 와 같은 규칙).
/// 겹치는 시각(가을)은 앞의 것.
fn local_ms(t: NaiveDateTime) -> Result<i64, String> {
    Local
        .from_local_datetime(&t)
        .earliest()
        .map(|d| d.timestamp_millis())
        .ok_or_else(|| format!("이 시간대엔 없는 시각이에요(서머타임): {}", t.format("%Y-%m-%dT%H:%M")))
}

fn day_start_ms(d: NaiveDate) -> Result<i64, String> {
    // 자정이 없는 날(자정에 서머타임이 바뀌는 곳)은 그날의 첫 시각.
    local_ms(d.and_time(NaiveTime::MIN)).or_else(|_| local_ms(d.and_hms_opt(1, 0, 0).unwrap_or_default()))
}

fn fmt_ms(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .earliest()
        .map(|d| d.format("%Y-%m-%dT%H:%M").to_string())
        .unwrap_or_default()
}

fn ms_date(ms: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ms).earliest().map(|d| d.date_naive()).unwrap_or_default()
}

/// 기간 인자 [from, to] (둘 다 포함, 없으면 오늘 / from) → 날짜 쌍. 길이 상한을 건다.
fn day_range(args: &Value, now: NaiveDateTime, max_days: u64) -> Result<(NaiveDate, NaiveDate), String> {
    let get = |k: &str| args.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty());
    let from = get("from").map(parse_day).transpose()?.unwrap_or(now.date());
    let to = get("to").map(parse_day).transpose()?.unwrap_or(from);
    if to < from {
        return Err("to 가 from 보다 앞이에요".into());
    }
    if (to - from).num_days() as u64 >= max_days {
        return Err(format!("한 번에 {max_days}일까지만 볼 수 있어요"));
    }
    Ok((from, to))
}

/// AI 에게 보이는 일정 한 건. 종일의 끝은 **포함**(사람 말과 같게) — 저장은 배타라 여기서 하루 뺀다.
fn event_json(e: &Event) -> Value {
    if e.all_day {
        let start = e.start_date.as_deref().and_then(|s| parse_day(s).ok()).unwrap_or_default();
        let end = e
            .end_date
            .as_deref()
            .and_then(|s| parse_day(s).ok())
            .and_then(|d| d.checked_sub_days(Days::new(1)))
            .unwrap_or(start);
        json!({
            "id": e.id, "title": e.title, "all_day": true,
            "start": start.format("%Y-%m-%d").to_string(), "end": end.format("%Y-%m-%d").to_string(),
            "weekday": weekday(start), "notes": e.notes,
        })
    } else {
        let s = e.start_at.unwrap_or_default();
        json!({
            "id": e.id, "title": e.title, "all_day": false,
            "start": fmt_ms(s), "end": fmt_ms(e.end_at.unwrap_or(s)),
            "weekday": weekday(ms_date(s)), "notes": e.notes,
        })
    }
}

// ── 도구 ────────────────────────────────────────────────────

fn agenda(store: &Store, args: &Value, now: NaiveDateTime) -> Result<Value, String> {
    let (from, to) = day_range(args, now, AGENDA_MAX_DAYS)?;
    let end = to.checked_add_days(Days::new(1)).ok_or("날짜가 너무 멀어요")?;
    let events = store.list_events_for_ai(day_start_ms(from)?, day_start_ms(end)?)?;
    Ok(json!({
        "from": from.format("%Y-%m-%d").to_string(),
        "to": to.format("%Y-%m-%d").to_string(),
        "events": events.iter().map(event_json).collect::<Vec<_>>(),
    }))
}

fn hhmm(args: &Value, key: &str, default: NaiveTime) -> Result<NaiveTime, String> {
    match args.get(key).and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        None => Ok(default),
        Some(s) => NaiveTime::parse_from_str(s.trim(), "%H:%M").map_err(|_| format!("{key} 는 HH:MM 으로 주세요")),
    }
}

/// 빈 시간 — 날마다 [day_start, day_end) 안에서 시각 일정이 없는 틈 중 `duration_minutes` 이상인 것.
/// 오늘은 지금 이후만. 틈은 통째로 돌려준다(길이에 맞춰 자르지 않는다 — 고르는 건 AI 와 사람).
fn free_time(store: &Store, args: &Value, now: NaiveDateTime, now_ms: i64) -> Result<Value, String> {
    let (from, to) = day_range(args, now, FREE_MAX_DAYS)?;
    let dur = args.get("duration_minutes").and_then(Value::as_i64).unwrap_or(30);
    if !(5..=24 * 60).contains(&dur) {
        return Err("duration_minutes 는 5~1440 이에요".into());
    }
    let ds = hhmm(args, "day_start", NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default())?;
    let de = hhmm(args, "day_end", NaiveTime::from_hms_opt(18, 0, 0).unwrap_or_default())?;
    if de <= ds {
        return Err("day_end 가 day_start 보다 앞이에요".into());
    }
    let mut slots = Vec::new();
    let mut d = from;
    'days: while d <= to {
        // 서머타임 틈에 걸린 경계는 그날을 건너뛴다(한국엔 없다 — 드문 날 하나를 틀리게 계산하느니 빼는 쪽).
        if let (Ok(ws), Ok(we)) = (local_ms(d.and_time(ds)), local_ms(d.and_time(de))) {
            let mut cursor = ws.max(now_ms);
            let mut busy = store.busy_between(ws, we)?;
            busy.sort();
            let mut gaps = Vec::new();
            for (s, e) in busy {
                if s > cursor {
                    gaps.push((cursor, s.min(we)));
                }
                cursor = cursor.max(e);
            }
            if cursor < we {
                gaps.push((cursor, we));
            }
            for (s, e) in gaps {
                if e - s >= dur * MIN {
                    slots.push(json!({ "start": fmt_ms(s), "end": fmt_ms(e), "weekday": weekday(d), "minutes": (e - s) / MIN }));
                    if slots.len() >= FREE_MAX_SLOTS {
                        break 'days;
                    }
                }
            }
        }
        d = match d.succ_opt() {
            Some(n) => n,
            None => break,
        };
    }
    Ok(json!({
        "from": from.format("%Y-%m-%d").to_string(),
        "to": to.format("%Y-%m-%d").to_string(),
        "slots": slots,
    }))
}

/// 제안 인자 → 일정 입력. `start` 가 날짜뿐이면 종일, 시각이 있으면 시각 일정.
/// 끝이 없으면 시각 일정은 1시간, 종일은 그날 하루. 종일의 `end` 는 **포함**(마지막 날).
fn propose_input(args: &Value) -> Result<EventInput, String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let title = s("title").ok_or("title 이 필요해요")?.to_string();
    let start = s("start").ok_or("start 가 필요해요")?;
    let notes = args.get("notes").and_then(Value::as_str).unwrap_or("").to_string();
    let alert_min = match args.get("alert_minutes") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or("alert_minutes 는 숫자예요")?),
    };
    let mut input = EventInput {
        title,
        notes,
        all_day: false,
        start_at: None,
        end_at: None,
        start_date: None,
        end_date: None,
        alert_min,
        private: false,
    };
    if start.len() == 10 {
        let sd = parse_day(start)?;
        let last = s("end").map(parse_day).transpose()?.unwrap_or(sd);
        if last < sd {
            return Err("끝나는 날이 시작보다 앞이에요".into());
        }
        let end = last.checked_add_days(Days::new(1)).ok_or("날짜가 너무 멀어요")?;
        input.all_day = true;
        input.start_date = Some(sd.format("%Y-%m-%d").to_string());
        input.end_date = Some(end.format("%Y-%m-%d").to_string());
    } else {
        let st = parse_local(start)?;
        let start_ms = local_ms(st)?;
        // 끝이 없으면 **실제로** 한 시간 뒤(순간 + 1시간). 벽시계에 1시간을 더하면 서머타임 날에 없는 시각이 되거나 두 시간이 된다(코덱스 개발 4).
        let end_ms = match s("end") {
            Some(e) => local_ms(parse_local(e)?)?,
            None => start_ms + 60 * MIN,
        };
        input.start_at = Some(start_ms);
        input.end_at = Some(end_ms);
    }
    Ok(input)
}

fn proposal_json(store: &Store, p: &Proposal) -> Result<Value, String> {
    let e = &p.event;
    let shown = Event {
        id: 0,
        title: e.title.clone(),
        notes: e.notes.clone(),
        all_day: e.all_day,
        start_at: e.start_at,
        end_at: e.end_at,
        start_date: e.start_date.clone(),
        end_date: e.end_date.clone(),
        alert_min: e.alert_min,
        private: false,
        origin: "mcp".into(),
        updated_at: p.created_at,
    };
    let mut ev = event_json(&shown);
    if let Some(o) = ev.as_object_mut() {
        o.remove("id");
    }
    let mut out = json!({
        "proposal_id": p.id,
        "status": p.status,
        "event": ev,
        "conflicts": conflicts(store, e, None)?,
    });
    if let Some(id) = p.event_id {
        out["event_id"] = json!(id);
    }
    Ok(out)
}

/// `get_proposal` 의 답. 기다리는·거절·만료된 제안은 AI 가 보낸 그대로를 보여 주지만, **받은 뒤엔 실제 일정을 본다** —
/// 사람이 고치며 «AI 에게 숨기기» 를 켰거나 지웠으면 내용은 안 나가고 상태만 나간다. 제안 번호는 순서대로라
/// 다른 AI 가 번호를 짐작해 물을 수 있다(코덱스 개발 4) — 비공개로 받은 일정이 이 문으로 새지 않게.
fn proposal_status_json(store: &Store, p: &Proposal) -> Result<Value, String> {
    if p.status != "approved" {
        return proposal_json(store, p);
    }
    let mut out = json!({ "proposal_id": p.id, "status": p.status });
    match p.event_id.map(|id| store.get_event(id)).transpose()?.flatten() {
        Some(e) if !e.private => {
            // 승인 결과에도 겹침을 싣는다 — 60초 안에 승인되면 AI 는 이 답만 받는다(코덱스 개발 4 2차). 자기 자신은 뺀다.
            let input = EventInput {
                title: e.title.clone(),
                notes: String::new(),
                all_day: e.all_day,
                start_at: e.start_at,
                end_at: e.end_at,
                start_date: e.start_date.clone(),
                end_date: e.end_date.clone(),
                alert_min: None,
                private: false,
            };
            out["conflicts"] = json!(conflicts(store, &input, Some(e.id))?);
            out["event_id"] = json!(e.id);
            out["event"] = event_json(&e);
        }
        _ => {}
    }
    Ok(out)
}

/// `propose_errand` 인자 → 부탁 제안. 시각은 일정 제안과 같은 로컬 벽시계 글자(`start`)다.
fn errand_proposal_input(args: &Value) -> Result<ErrandProposalInput, String> {
    let prompt = args.get("prompt").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or("prompt 가 필요해요")?;
    let start = args.get("start").and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()).ok_or("start 가 필요해요")?;
    if start.len() == 10 {
        return Err("start 는 시각이 있어야 해요(YYYY-MM-DDTHH:MM) — 날짜만으론 언제 돌지 몰라요".into());
    }
    let start_at = local_ms(parse_local(start)?)?;
    let allowed_tools = match args.get("allow_web_search") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => "",
        Some(Value::Bool(true)) => "WebSearch",
        Some(_) => return Err("allow_web_search 는 true/false 예요".into()),
    };
    debug_assert!(TOOL_CHOICES.contains(&allowed_tools));
    let late = match args.get("if_missed").and_then(Value::as_str) {
        None | Some("run") => "run",
        Some("skip") => "skip",
        Some(_) => return Err("if_missed 는 run 또는 skip 이에요".into()),
    };
    let from_run_id = match args.get("from_run_id") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_i64().ok_or("from_run_id 는 숫자예요")?),
    };
    let target = match args.get("target").and_then(Value::as_str) {
        None | Some("claude") => "claude",
        Some("codex") => "codex",
        Some(_) => return Err("target 은 claude 또는 codex 예요".into()),
    };
    if args.get("target").is_some_and(|v| !v.is_null() && !v.is_string()) {
        return Err("target 은 claude 또는 codex 예요".into());
    }
    Ok(ErrandProposalInput {
        errand: ErrandInput {
            prompt: prompt.to_string(),
            start_at,
            allowed_tools: allowed_tools.into(),
            late: late.into(),
            target: target.into(),
            ..ErrandInput::default()
        },
        from_run_id,
    })
}

/// 부탁 제안의 답. 기다리는·거절·만료는 AI 가 보낸 그대로를 보여 주고, **받은 뒤엔 내용 없이 상태만** — 머리 주석.
fn errand_proposal_json(store: &Store, p: &ErrandProposal) -> Result<Value, String> {
    let mut out = json!({ "proposal_id": p.id, "status": p.status, "depth": p.depth });
    if p.status != "approved" {
        out["errand"] = json!({
            "prompt": p.prompt,
            "start": fmt_ms(p.start_at),
            "weekday": weekday(ms_date(p.start_at)),
            "allow_web_search": p.allowed_tools == "WebSearch",
            "if_missed": p.late,
            "target": p.target,
        });
        return Ok(out);
    }
    // 사람이 지웠으면 부탁은 없다 — 상태만 «approved».
    if let Some(e) = p.errand_id.map(|id| store.get_errand(id)).transpose()?.flatten() {
        out["errand_id"] = json!(e.id);
        out["run_status"] = json!(e.run.as_ref().map_or("waiting", |r| r.status.as_str()));
        if let Some(r) = &e.run {
            out["run_id"] = json!(r.id);
        }
    }
    Ok(out)
}

/// 제안과 겹치는 (AI 에게 보여도 되는) 일정 제목. 비공개는 «비공개 일정» 으로만.
pub(crate) fn conflicts(store: &Store, e: &EventInput, skip: Option<i64>) -> Result<Vec<String>, String> {
    let (from, to) = match (e.start_at, e.end_at, e.start_date.as_deref(), e.end_date.as_deref()) {
        (Some(s), Some(x), _, _) => (s, x.max(s + 1)),
        (_, _, Some(s), Some(x)) => (day_start_ms(parse_day(s)?)?, day_start_ms(parse_day(x)?)?),
        _ => return Ok(vec![]),
    };
    Ok(store
        .list_events(from, to)?
        .into_iter()
        // 시각 제안엔 시각 일정만 겹침으로 친다(종일 표시는 시간을 막지 않는다 — busy_between 과 같은 규칙).
        .filter(|x| e.all_day || !x.all_day)
        .filter(|x| Some(x.id) != skip)
        .map(|x| if x.private { "비공개 일정".to_string() } else { x.title })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).earliest().unwrap() // 목요일
    }

    fn req(op: &str, args: Value) -> Request {
        Request { client: "claude-code".into(), instance: String::new(), op: op.into(), args }
    }

    fn at(s: &str) -> i64 {
        local_ms(parse_local(s).unwrap()).unwrap()
    }

    fn timed(title: &str, s: &str, e: &str, private: bool) -> EventInput {
        EventInput {
            title: title.into(),
            notes: String::new(),
            all_day: false,
            start_at: Some(at(s)),
            end_at: Some(at(e)),
            start_date: None,
            end_date: None,
            alert_min: None,
            private,
        }
    }

    #[test]
    fn agenda_hides_private_and_speaks_local_time() {
        let s = Store::open_in_memory();
        s.create_event(&timed("회의", "2026-10-01T15:00", "2026-10-01T16:00", false)).unwrap();
        s.create_event(&timed("병원", "2026-10-01T10:00", "2026-10-01T11:00", true)).unwrap();
        s.create_event(&timed("내일", "2026-10-02T10:00", "2026-10-02T11:00", false)).unwrap();
        let out = handle(&s, &req("agenda", json!({})), now()).unwrap();
        let ev = out["events"].as_array().unwrap();
        assert_eq!(ev.len(), 1, "비공개와 내일은 없다: {out}");
        assert_eq!(ev[0]["start"], "2026-10-01T15:00");
        assert_eq!(ev[0]["weekday"], "Thu");
        assert_eq!(out["now"], "2026-10-01T08:00 (Thu)");
        assert!(!out.to_string().contains("병원"));
        let two = handle(&s, &req("agenda", json!({"from":"2026-10-01","to":"2026-10-02"})), now()).unwrap();
        assert_eq!(two["events"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn agenda_limits_and_shapes() {
        let s = Store::open_in_memory();
        assert!(handle(&s, &req("agenda", json!({"from":"2026-10-03","to":"2026-10-01"})), now()).is_err());
        assert!(handle(&s, &req("agenda", json!({"from":"2026-01-01","to":"2026-12-31"})), now()).is_err());
        assert!(handle(&s, &req("agenda", json!({"from":"2026-10-1"})), now()).is_err(), "자릿수 고정");
    }

    #[test]
    fn all_day_end_is_inclusive_outward() {
        let s = Store::open_in_memory();
        let p = handle(&s, &req("propose", json!({"title":"여행","start":"2026-10-03","end":"2026-10-05"})), now()).unwrap();
        assert_eq!(p["event"]["end"], "2026-10-05");
        let stored = s.get_proposal(p["proposal_id"].as_i64().unwrap()).unwrap().unwrap();
        assert_eq!(stored.event.end_date.as_deref(), Some("2026-10-06"), "저장은 배타");
        let one = handle(&s, &req("propose", json!({"title":"쉼","start":"2026-10-09"})), now()).unwrap();
        assert_eq!((one["event"]["start"].as_str(), one["event"]["end"].as_str()), (Some("2026-10-09"), Some("2026-10-09")));
        assert_eq!(one["event"]["weekday"], "Fri");
    }

    #[test]
    fn propose_only_proposes() {
        let s = Store::open_in_memory();
        s.create_event(&timed("원래 약속", "2026-10-02T15:30", "2026-10-02T16:30", false)).unwrap();
        let p = handle(&s, &req("propose", json!({"title":"치과","start":"2026-10-02T15:00"})), now()).unwrap();
        assert_eq!(p["status"], "pending");
        assert_eq!(p["event"]["end"], "2026-10-02T16:00", "끝이 없으면 1시간");
        assert_eq!(p["conflicts"], json!(["원래 약속"]));
        assert_eq!(s.list_events_for_ai(0, i64::MAX).unwrap().len(), 1, "일정은 늘지 않았다");
        let id = p["proposal_id"].as_i64().unwrap();
        let q = handle(&s, &req("proposal", json!({"id": id})), now()).unwrap();
        assert_eq!(q["status"], "pending");
        // 소켓엔 승인 문이 없다.
        for op in ["approve", "approve_proposal", "create", "delete", "update"] {
            assert!(handle(&s, &req(op, json!({"id": id})), now()).is_err(), "{op}");
        }
        s.approve_proposal(id, None).unwrap();
        let q = handle(&s, &req("proposal", json!({"id": id})), now()).unwrap();
        assert_eq!(q["status"], "approved");
        assert!(q["event_id"].is_i64());
    }

    /// 지금부터 이틀 뒤 09:00 (로컬 글자, 요일) — 실제 시계 기준이라 «지난 시각»·«1년 넘게» 검사에 안 걸린다.
    fn soon_local() -> (String, &'static str) {
        let d = Local::now().date_naive().checked_add_days(Days::new(2)).unwrap();
        (format!("{}T09:00", d.format("%Y-%m-%d")), weekday(d))
    }

    #[test]
    fn errand_proposal_only_proposes_and_status_has_no_content() {
        let s = Store::open_in_memory();
        let (when, wd) = soon_local();
        let args = json!({"prompt":"어제 커밋 정리해 줘","start":when,"allow_web_search":true,"if_missed":"skip"});
        let p = handle(&s, &req("propose_errand", args), now()).unwrap();
        assert_eq!(p["status"], "pending");
        assert_eq!(p["errand"]["start"], when.as_str());
        assert_eq!(p["errand"]["weekday"], wd);
        assert_eq!((p["errand"]["allow_web_search"].clone(), p["errand"]["if_missed"].clone()), (json!(true), json!("skip")));
        assert!(s.list_errands(0, i64::MAX).unwrap().is_empty(), "부탁은 늘지 않았다");
        assert!(s.due_errands(i64::MAX).unwrap().is_empty());
        let id = p["proposal_id"].as_i64().unwrap();
        // 소켓엔 승인 문이 없다.
        for op in ["approve_errand", "approve_errand_proposal", "run_errand", "create_errand", "delete_errand"] {
            assert!(handle(&s, &req(op, json!({"id": id})), now()).is_err(), "{op}");
        }
        // 사람이 고쳐서 받았다 — AI 가 받는 답엔 글도 답도 없다.
        let mine = ErrandInput { prompt: "비밀 내용".into(), start_at: crate::store::now_ms() + 60_000, ..ErrandInput::default() };
        let e = s.approve_errand_proposal(id, Some(&mine)).unwrap();
        let q = handle(&s, &req("errand_proposal", json!({"id": id})), now()).unwrap();
        assert_eq!((q["status"].clone(), q["run_status"].clone()), (json!("approved"), json!("waiting")));
        assert!(q.get("errand").is_none() && !q.to_string().contains("비밀"), "{q}");
        let run = s.begin_run(e.id, "비밀 내용", 0, None).unwrap();
        let q = handle(&s, &req("errand_proposal", json!({"id": id})), now()).unwrap();
        assert_eq!((q["run_status"].clone(), q["run_id"].clone()), (json!("running"), json!(run)));
        // 사람이 부탁을 지우면 상태만.
        s.delete_errand(e.id).ok();
        s.finish_run(run, &crate::errands::Outcome { status: "done", exit_code: Some(0), response: Some("답".into()), stderr: None, session_id: None }).unwrap();
        s.delete_errand(e.id).unwrap();
        let q = handle(&s, &req("errand_proposal", json!({"id": id})), now()).unwrap();
        assert!(q.get("run_status").is_none() && !q.to_string().contains("답\""), "{q}");
    }

    #[test]
    fn errand_proposal_rejects_bad_args() {
        let s = Store::open_in_memory();
        let when = soon_local().0;
        let ok = || json!({"prompt":"안녕","start":when});
        let with = |k: &str, v: Value| {
            let mut a = ok();
            a[k] = v;
            a
        };
        assert!(handle(&s, &req("propose_errand", ok()), now()).is_ok());
        for bad in [
            json!({"start":when}),
            json!({"prompt":"  ","start":when}),
            json!({"prompt":"안녕"}),
            with("start", json!(&when[..10])),
            with("start", json!(format!("{when}Z"))),
            with("start", json!("2000-01-02T09:00")),
            with("allow_web_search", json!("yes")),
            with("if_missed", json!("later")),
            with("from_run_id", json!("x")),
            with("from_run_id", json!(12345)),
        ] {
            assert!(handle(&s, &req("propose_errand", bad.clone()), now()).is_err(), "{bad}");
        }
    }

    #[test]
    fn approved_private_proposal_does_not_leak() {
        let s = Store::open_in_memory();
        let p = handle(&s, &req("propose", json!({"title":"정신과","start":"2026-10-02T15:00","notes":"상담"})), now()).unwrap();
        let id = p["proposal_id"].as_i64().unwrap();
        // 사람이 고쳐서 받으며 «AI 에게 숨기기» 를 켰다.
        let mut edited = timed("정신과 상담", "2026-10-02T15:00", "2026-10-02T16:00", true);
        edited.notes = "비밀".into();
        s.approve_proposal(id, Some(&edited)).unwrap();
        let q = handle(&s, &req("proposal", json!({"id": id})), now()).unwrap();
        assert_eq!(q["status"], "approved");
        assert!(!q.to_string().contains("정신과") && !q.to_string().contains("상담"), "{q}");
        assert!(q.get("event_id").is_none());
        // 숨기지 않고 받은 건 고친 실제 일정을 보여 준다.
        let p2 = handle(&s, &req("propose", json!({"title":"치과","start":"2026-10-03T15:00"})), now()).unwrap();
        let id2 = p2["proposal_id"].as_i64().unwrap();
        s.approve_proposal(id2, Some(&timed("치과 (옮김)", "2026-10-03T16:00", "2026-10-03T17:00", false))).unwrap();
        s.create_event(&timed("회의", "2026-10-03T16:30", "2026-10-03T17:30", false)).unwrap();
        let q2 = handle(&s, &req("proposal", json!({"id": id2})), now()).unwrap();
        assert_eq!(q2["event"]["title"], "치과 (옮김)");
        assert_eq!(q2["conflicts"], json!(["회의"]), "승인 답에도 겹침, 자기 자신은 빼고");
    }

    #[test]
    fn default_end_is_a_real_hour() {
        let s = Store::open_in_memory();
        let p = handle(&s, &req("propose", json!({"title":"x","start":"2026-10-02T15:00"})), now()).unwrap();
        let e = s.get_proposal(p["proposal_id"].as_i64().unwrap()).unwrap().unwrap().event;
        assert_eq!(e.end_at.unwrap() - e.start_at.unwrap(), 60 * MIN);
    }

    #[test]
    fn propose_rejects_bad_times() {
        let s = Store::open_in_memory();
        let bad = [
            json!({"title":"x","start":"2026-10-02T15:00Z"}),
            json!({"title":"x","start":"2026-10-02T15:00+09:00"}),
            json!({"title":"x","start":"내일 3시"}),
            json!({"title":"x","start":"2026-10-02T15:00","end":"2026-10-02T14:00"}),
            json!({"title":"x","start":"2026-10-05","end":"2026-10-03"}),
            json!({"title":"","start":"2026-10-02T15:00"}),
            json!({"start":"2026-10-02T15:00"}),
            json!({"title":"x","start":"2026-10-02T15:00","alert_minutes":7}),
            json!({"title":"x","start":"2026-10-02T15:00","alert_minutes":"10"}),
        ];
        for a in bad {
            assert!(handle(&s, &req("propose", a.clone()), now()).is_err(), "{a}");
        }
        assert!(s.pending_proposals().unwrap().is_empty());
        let ok = handle(&s, &req("propose", json!({"title":"x","start":"2026-10-02T15:00:00","alert_minutes":10})), now());
        assert!(ok.is_ok());
    }

    #[test]
    fn private_conflict_is_named_only_as_private() {
        let s = Store::open_in_memory();
        s.create_event(&timed("정신과", "2026-10-02T15:00", "2026-10-02T16:00", true)).unwrap();
        let p = handle(&s, &req("propose", json!({"title":"치과","start":"2026-10-02T15:00"})), now()).unwrap();
        assert_eq!(p["conflicts"], json!(["비공개 일정"]));
    }

    #[test]
    fn free_time_walks_gaps() {
        let s = Store::open_in_memory();
        s.create_event(&timed("a", "2026-10-02T10:00", "2026-10-02T11:00", false)).unwrap();
        s.create_event(&timed("b", "2026-10-02T10:30", "2026-10-02T12:00", true)).unwrap(); // 겹침 + 비공개도 바쁨
        s.create_event(&timed("c", "2026-10-02T13:00", "2026-10-02T13:20", false)).unwrap();
        let out = handle(&s, &req("free_time", json!({"from":"2026-10-02","duration_minutes":60})), now()).unwrap();
        let got: Vec<(String, String)> = out["slots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["start"].as_str().unwrap().into(), x["end"].as_str().unwrap().into()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("2026-10-02T09:00".into(), "2026-10-02T10:00".into()),
                ("2026-10-02T12:00".into(), "2026-10-02T13:00".into()), // 딱 60분도 든다
                ("2026-10-02T13:20".into(), "2026-10-02T18:00".into()),
            ]
        );
    }

    #[test]
    fn free_time_includes_exact_fit_and_skips_past() {
        let s = Store::open_in_memory();
        s.create_event(&timed("점심 전", "2026-10-01T09:00", "2026-10-01T12:00", false)).unwrap();
        s.create_event(&timed("오후", "2026-10-01T13:00", "2026-10-01T18:00", false)).unwrap();
        let later = Local.with_ymd_and_hms(2026, 10, 1, 12, 10, 0).earliest().unwrap();
        let out = handle(&s, &req("free_time", json!({"duration_minutes":30})), later).unwrap();
        assert_eq!(out["slots"], json!([{"start":"2026-10-01T12:10","end":"2026-10-01T13:00","weekday":"Thu","minutes":50}]));
        let out = handle(&s, &req("free_time", json!({"duration_minutes":60})), later).unwrap();
        assert_eq!(out["slots"], json!([]));
        assert!(handle(&s, &req("free_time", json!({"day_start":"18:00","day_end":"09:00"})), later).is_err());
        assert!(handle(&s, &req("free_time", json!({"duration_minutes":0})), later).is_err());
    }

    #[test]
    fn presence_counts_processes_and_expires() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let p = Presence::default();
        assert!(p.touch("11", "claude-code", at(0)), "처음 붙음 = 바뀜");
        assert!(!p.touch("11", "claude-code", at(1_000)), "같은 소식 = 안 바뀜");
        assert!(!p.touch("22", "claude-code", at(2_000)), "같은 이름의 두 번째 세션 = 목록은 그대로");
        assert!(!p.leave("11", at(3_000)), "하나가 나가도 다른 세션이 남았다");
        assert!(!p.touch("11", "claude-code", at(3_100)), "bye 뒤에 늦게 처리된 hello 는 무시");
        assert_eq!(p.clients(at(3_000)), vec!["claude-code"]);
        assert!(p.touch("33", "codex-mcp-client", at(4_000)));
        assert_eq!(p.clients(at(4_000)), vec!["claude-code", "codex-mcp-client"]);
        assert_eq!(p.clients(at(2_000) + ALIVE + Duration::from_millis(1)), vec!["codex-mcp-client"], "소식 끊긴 세션은 빠진다");
        assert!(p.leave("33", at(5_000)));
    }

    #[test]
    fn presence_is_bounded() {
        let now = Instant::now();
        let p = Presence::default();
        for i in 0..PRESENCE_MAX + 10 {
            p.touch(&format!("{i}"), &format!("c{i}"), now);
        }
        assert_eq!(p.clients(now).len(), PRESENCE_MAX);
        p.touch(&"x".repeat(10_000), "long", now);
        assert!(p.lock().seen.keys().all(|k| k.chars().count() <= INSTANCE_MAX));
    }

    #[test]
    fn slow_request_is_cut_at_the_deadline() {
        // 한 바이트씩 흘리는 연결도 시한이 지나면 끊는다 — 읽기 시간 제한만으론 영영 붙들린다.
        let (mut a, mut b) = UnixStream::pair().unwrap();
        let t = Instant::now();
        let writer = std::thread::spawn(move || {
            for _ in 0..20 {
                if b.write_all(b"{").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        assert_eq!(read_line_by(&mut a, REQUEST_MAX, Instant::now() + Duration::from_millis(500)), None);
        assert!(t.elapsed() < Duration::from_millis(1500));
        drop(a);
        writer.join().unwrap();
        let (mut a, mut b) = UnixStream::pair().unwrap();
        b.write_all(b"{\"op\":\"x\"}\nrest").unwrap();
        assert_eq!(read_line_by(&mut a, REQUEST_MAX, Instant::now() + Duration::from_secs(1)).as_deref(), Some("{\"op\":\"x\"}"));
    }

    #[test]
    fn socket_round_trip_and_stale_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = socket_path(dir.path());
        // 죽은 앱이 남긴 소켓 파일(연결 안 받음)은 지우고 연다.
        drop(UnixListener::bind(&path).unwrap());
        let store = Arc::new(Store::open_in_memory());
        let presence = Arc::new(Presence::default());
        let hits = Arc::new(std::sync::Mutex::new(Vec::new()));
        let h = hits.clone();
        serve(store.clone(), presence.clone(), dir.path(), Arc::new(move |c| h.lock().unwrap().push(c))).unwrap();
        let ask = |line: &str| {
            let mut c = UnixStream::connect(&path).unwrap();
            writeln!(c, "{line}").unwrap();
            let mut out = String::new();
            BufReader::new(c).read_line(&mut out).unwrap();
            serde_json::from_str::<Value>(&out).unwrap()
        };
        let r = ask(r#"{"client":"t","instance":"1","op":"propose","args":{"title":"치과","start":"2030-01-02T15:00"}}"#);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["data"]["status"], "pending");
        assert_eq!(ask("not json")["ok"], false);
        assert_eq!(ask(r#"{"client":"t","instance":"1","op":"approve","args":{"id":1}}"#)["ok"], false);
        assert_eq!(*hits.lock().unwrap(), vec![Change::Presence, Change::Proposal], "붙음 한 번 + 새 제안에만 팝오버");
        assert_eq!(ask(r#"{"client":"t","instance":"1","op":"bye"}"#)["ok"], true);
        assert!(presence.clients(Instant::now()).is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // 살아 있는 소켓은 빼앗지 않는다.
        assert!(serve(store, presence, dir.path(), Arc::new(|_| {})).is_err());
    }
}
