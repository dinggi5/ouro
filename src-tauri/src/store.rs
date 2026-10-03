// 저장소 — `~/.ouro/ouro.db` (SQLite). 일정·부탁·실행이 전부 여기 산다(PLAN §4·§6).
//
// 설계 결정:
//   · **쓰는 곳은 앱 하나.** MCP 사이드카도 DB 를 직접 열지 않고 소켓으로 앱에 묻는다(개발 4).
//     그래서 연결 하나를 Mutex 로 감싸 두는 것으로 충분하다 — 풀·동시 쓰기 조정이 필요 없다.
//   · 일정과 부탁은 **같은 표의 두 종류**(`items.kind`). 부탁에만 있는 칸은 `errands` 에 따로 둔다 —
//     일정 하나를 «부탁으로 바꾸기» 가 kind 한 칸 + errands 한 줄이 된다.
//   · 시간은 두 갈래다. 시각이 있는 일정 = `start_at`/`end_at`(UTC 밀리초, 절대 시각).
//     종일 일정 = `start_date`/`end_date`(로컬 달력 날짜 `YYYY-MM-DD`, **끝은 배타**, RFC 5545 DTEND 와 같은 규칙).
//     종일을 밀리초로 저장하면 다른 시간대로 가거나 서머타임이 끼는 순간 하루가 밀린다 — 종일은 «날짜» 지 «시각» 이 아니다.
//   · **비공개(`private`) 일정은 MCP 로 나가지 않는다**(PLAN §8). 막는 곳은 `list_events_for_ai` 하나 —
//     빈 시간 계산(`busy_between`)만 비공개도 «바쁨» 으로 센다 — 제목·메모는 안 나가지만 **빈 틈의 끝점으로 그 시각은 짐작된다**
//     (코덱스 개발 4). 알고 고른 값이다: 안 세면 AI 가 비공개 일정 위에 약속을 제안한다. 비공개 = «내용을 숨긴다» 지 «그 시간이 빈다» 가 아니다.
//   · AI 가 낸 일정은 `proposals` 에 **제안**으로만 들어간다. 일정이 되는 길은 사람이 누르는 `approve_proposal` 하나다
//     (CLAUDE.md 불변 규칙 — 소켓엔 승인 문이 없다, mcp.rs).
//   · 지우기는 **휴지통**(`deleted_at`). 팝오버의 «되돌리기» 가 되살릴 수 있어야 하고, 동기화가 없어
//     실수로 지운 일정은 되찾을 길이 백업뿐이다. 7일 지나면 켤 때 비운다(`purge_trash`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Days, Local, NaiveDate, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

/// 휴지통 보관 기간.
const TRASH_KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// 제목·메모 길이 상한(글자 수). 사람이 쓰는 칸이라 넉넉하되, MCP·가져오기로 들어올 거대한 글을 막는다.
const TITLE_MAX: usize = 200;
const NOTES_MAX: usize = 10_000;
/// 고를 수 있는 알림(분 전). 팝오버의 선택지와 같아야 한다(`src/lib/events.ts` 의 ALERTS_TIMED·ALERTS_ALL_DAY).
/// 종일 일정은 «그날 오전 9시» 에서 뺀다 — 0 = 당일 9시, 1440 = 전날 9시.
pub(crate) const ALERT_CHOICES: [i64; 8] = [0, 5, 10, 15, 30, 60, 120, 1440];
/// 종일 일정 알림의 기준 시각(로컬). 애플 캘린더의 기본값과 같다.
pub(crate) const ALL_DAY_ALERT_HOUR: u32 = 9;

/// 데이터 폴더. 앱을 지워도 남는다(Kura `~/.jigap` 과 같은 이유 — 앱 번들 안에 두면 업데이트·재설치에 날아간다).
///
/// 디버그 빌드에서만 `OURO_HOME` 으로 바꿀 수 있다 — 개발 중 실물 DB 를 더럽히지 않으려고.
/// 릴리스에선 환경변수로 저장 위치가 바뀌면 «내 일정이 사라졌다» 가 되므로 막는다.
pub(crate) fn data_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("OURO_HOME") {
        return Some(PathBuf::from(p));
    }
    dirs::home_dir().map(|h| h.join(".ouro"))
}

/// 일정 한 건 — 프론트로 가는 모양.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Event {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub all_day: bool,
    pub start_at: Option<i64>,
    pub end_at: Option<i64>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub alert_min: Option<i64>,
    /// 1 = MCP 로 안 나간다(PLAN §8).
    pub private: bool,
    /// 누가 만들었나 — `user`(팝오버) · `mcp`(AI 제안을 사람이 승인) · `import`.
    pub origin: String,
    pub updated_at: i64,
}

/// 만들기·고치기 입력. 모양 검사는 `validate` 가 한다 — 프론트를 믿지 않는다(개발 4 부터는 MCP 도 이 길로 온다).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventInput {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub all_day: bool,
    pub start_at: Option<i64>,
    pub end_at: Option<i64>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub alert_min: Option<i64>,
    #[serde(default)]
    pub private: bool,
}

/// 검사를 통과한 입력. 제목은 앞뒤 공백을 걷어 낸 값.
#[derive(Debug, Clone, PartialEq)]
struct Valid {
    title: String,
    notes: String,
    when: When,
    alert_min: Option<i64>,
    private: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum When {
    Timed { start: i64, end: i64 },
    AllDay { start: NaiveDate, end: NaiveDate },
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    // `%Y-%m-%d` 는 `2026-9-1` 도 받는다 — 문자열 비교로 범위를 찾으니 자릿수가 고정이어야 한다.
    if s.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// 입력 검사 (순수 — 테스트 가능). 틀리면 사람에게 보일 문장을 돌려준다.
fn validate(input: &EventInput) -> Result<Valid, String> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err("제목을 적어 주세요".into());
    }
    if title.chars().count() > TITLE_MAX {
        return Err(format!("제목은 {TITLE_MAX}자까지예요"));
    }
    if input.notes.chars().count() > NOTES_MAX {
        return Err(format!("메모는 {NOTES_MAX}자까지예요"));
    }
    let when = if input.all_day {
        if input.start_at.is_some() || input.end_at.is_some() {
            return Err("종일 일정에 시각이 섞여 있어요".into());
        }
        let (Some(s), Some(e)) = (input.start_date.as_deref(), input.end_date.as_deref()) else {
            return Err("날짜가 비어 있어요".into());
        };
        let (Some(start), Some(end)) = (parse_date(s), parse_date(e)) else {
            return Err("날짜 모양이 이상해요".into());
        };
        // 끝은 배타 — 하루짜리 종일 일정도 end = start + 1 이다.
        if end <= start {
            return Err("끝나는 날이 시작보다 앞이에요".into());
        }
        When::AllDay { start, end }
    } else {
        if input.start_date.is_some() || input.end_date.is_some() {
            return Err("시각 일정에 종일 날짜가 섞여 있어요".into());
        }
        let (Some(start), Some(end)) = (input.start_at, input.end_at) else {
            return Err("시각이 비어 있어요".into());
        };
        if end < start {
            return Err("끝나는 시각이 시작보다 앞이에요".into());
        }
        When::Timed { start, end }
    };
    if let Some(m) = input.alert_min {
        if !ALERT_CHOICES.contains(&m) {
            return Err("알림 시간이 이상해요".into());
        }
    }
    Ok(Valid {
        title: title.to_string(),
        notes: input.notes.clone(),
        when,
        alert_min: input.alert_min,
        private: input.private,
    })
}

/// 스키마 이력. `PRAGMA user_version` = 적용된 개수. **이미 나간 항목은 절대 고치지 않는다** —
/// 사용자 DB 는 옛 항목이 이미 돌아간 상태라, 바꿀 게 생기면 새 항목을 뒤에 붙인다.
pub(crate) const MIGRATIONS: &[&str] = &[
    // 1 — 개발 2: 일정·부탁·실행.
    "
    CREATE TABLE items (
        id          INTEGER PRIMARY KEY,
        kind        TEXT    NOT NULL CHECK (kind IN ('event', 'errand')),
        title       TEXT    NOT NULL,
        notes       TEXT    NOT NULL DEFAULT '',
        all_day     INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
        start_at    INTEGER,          -- UTC ms (시각 일정)
        end_at      INTEGER,
        start_date  TEXT,             -- YYYY-MM-DD (종일, 로컬 달력)
        end_date    TEXT,             -- 배타
        tz          TEXT    NOT NULL, -- 만들 때의 IANA 시간대. 반복(RRULE, v0.2) 전개의 기준
        rrule       TEXT,             -- v0.2
        alert_min   INTEGER,          -- NULL = 알림 없음
        private     INTEGER NOT NULL DEFAULT 0 CHECK (private IN (0, 1)), -- 1 = MCP 로 안 나간다(PLAN §8)
        origin      TEXT    NOT NULL DEFAULT 'user' CHECK (origin IN ('user', 'mcp', 'import')),
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        deleted_at  INTEGER,          -- 휴지통
        CHECK ((all_day = 0 AND start_at IS NOT NULL AND end_at IS NOT NULL AND end_at >= start_at
                AND start_date IS NULL AND end_date IS NULL)
            OR (all_day = 1 AND start_date IS NOT NULL AND end_date IS NOT NULL AND end_date > start_date
                AND start_at IS NULL AND end_at IS NULL))
    );
    CREATE INDEX items_start_at   ON items (start_at)   WHERE deleted_at IS NULL;
    CREATE INDEX items_start_date ON items (start_date) WHERE deleted_at IS NULL;

    -- 부탁에만 있는 칸(개발 5 에서 채운다). 바깥으로 나갈 문장은 사람이 쓰거나 승인한 것만(CLAUDE.md 불변 규칙).
    CREATE TABLE errands (
        item_id       INTEGER PRIMARY KEY REFERENCES items (id) ON DELETE CASCADE,
        target        TEXT    NOT NULL CHECK (target IN ('claude', 'codex')),
        prompt        TEXT    NOT NULL,
        workdir       TEXT,
        allowed_tools TEXT    NOT NULL DEFAULT '',
        approved_at   INTEGER,        -- AI 가 만든 부탁은 사람 승인 전 NULL → 안 돈다
        parent_run_id INTEGER REFERENCES runs (id) ON DELETE SET NULL,
        depth         INTEGER NOT NULL DEFAULT 0 -- 파생 깊이 ≤ 3 (PLAN §9-4)
    );

    -- 부탁 한 번 돈 기록. 보낸 원문을 그대로 남긴다(PLAN §9-6).
    CREATE TABLE runs (
        id          INTEGER PRIMARY KEY,
        item_id     INTEGER NOT NULL REFERENCES items (id) ON DELETE CASCADE,
        started_at  INTEGER NOT NULL,
        finished_at INTEGER,
        exit_code   INTEGER,
        sent_text   TEXT    NOT NULL,
        response    TEXT,
        stderr      TEXT,
        session_id  TEXT
    );
    CREATE INDEX runs_item ON runs (item_id);

    -- 이미 띄운 알림. (일정, 울릴 시각) 쌍이라 일정 시각을 고치면 새 알림으로 다시 걸린다.
    CREATE TABLE alerts_sent (
        item_id INTEGER NOT NULL REFERENCES items (id) ON DELETE CASCADE,
        fire_at INTEGER NOT NULL,
        sent_at INTEGER NOT NULL,
        PRIMARY KEY (item_id, fire_at)
    );
    ",
    // 2 — 개발 3: 빠른 입력이 못 알아들은 횟수(유형별 개수만, 글은 저장하지 않는다 — PLAN §7).
    //     로컬 모델을 붙일지 사장이 판단할 때 보는 숫자다.
    "
    CREATE TABLE parse_misses (
        kind    TEXT    PRIMARY KEY,
        count   INTEGER NOT NULL DEFAULT 0,
        last_at INTEGER NOT NULL
    );
    ",
    // 3 — 개발 4: AI 가 MCP 로 낸 일정 제안. 사람이 승인해야 `items` 에 일정이 생긴다(`approve_proposal`).
    //     일정 칸은 `items` 와 같은 모양(검사도 같은 `validate`). 결정된 제안은 남겨 둔다 — «AI 가 무엇을 냈고 내가 무엇을 받았나».
    "
    CREATE TABLE proposals (
        id          INTEGER PRIMARY KEY,
        client      TEXT    NOT NULL,  -- MCP 클라이언트가 스스로 밝힌 이름(믿지 않는다 — 보여 주기만)
        title       TEXT    NOT NULL,
        notes       TEXT    NOT NULL DEFAULT '',
        all_day     INTEGER NOT NULL CHECK (all_day IN (0, 1)),
        start_at    INTEGER,
        end_at      INTEGER,
        start_date  TEXT,
        end_date    TEXT,
        alert_min   INTEGER,
        status      TEXT    NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
        created_at  INTEGER NOT NULL,
        decided_at  INTEGER,
        event_id    INTEGER REFERENCES items (id) ON DELETE SET NULL
    );
    CREATE INDEX proposals_pending ON proposals (created_at) WHERE status = 'pending';
    ",
    // 4 — 개발 5: 부탁이 돌고 답이 돌아온다. 놓친 부탁을 «늦게 실행 / 건너뜀» 중 부탁마다 고르고(PLAN §9-3),
    //     실행 기록에 상태·늦은 정도·읽음을 둔다. 건너뛴 부탁도 기록 한 줄을 남겨 다시 걸리지 않게 한다.
    "
    ALTER TABLE errands ADD COLUMN late TEXT NOT NULL DEFAULT 'run' CHECK (late IN ('run', 'skip'));
    ALTER TABLE runs ADD COLUMN status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'done', 'failed', 'stopped', 'skipped'));
    ALTER TABLE runs ADD COLUMN late_ms INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE runs ADD COLUMN read_at INTEGER;
    ",
    // 5 — 개발 6: AI 가 MCP 로 낸 «부탁 제안». 사람이 승인해야 `items`+`errands` 가 생기고(approved_at 이 찍힌 채로) 그제야 돈다.
    //     `parent_run_id`·`depth` = 어느 실행의 답에서 이어진 부탁인가(PLAN §9-4: 깊이 ≤ 3). 결정된 제안은 남겨 둔다.
    "
    CREATE TABLE errand_proposals (
        id            INTEGER PRIMARY KEY,
        client        TEXT    NOT NULL,
        prompt        TEXT    NOT NULL,
        start_at      INTEGER NOT NULL,
        allowed_tools TEXT    NOT NULL DEFAULT '',
        late          TEXT    NOT NULL DEFAULT 'run' CHECK (late IN ('run', 'skip')),
        parent_run_id INTEGER REFERENCES runs (id) ON DELETE SET NULL,
        depth         INTEGER NOT NULL DEFAULT 0,
        status        TEXT    NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
        created_at    INTEGER NOT NULL,
        decided_at    INTEGER,
        errand_id     INTEGER REFERENCES items (id) ON DELETE SET NULL
    );
    CREATE INDEX errand_proposals_pending ON errand_proposals (created_at) WHERE status = 'pending';
    ",
];

/// 한꺼번에 기다릴 수 있는 제안 수. AI 가 고리에 빠져 수백 개를 쌓아 팝오버를 덮지 않게.
const PENDING_MAX: i64 = 20;
/// 이만큼 아무도 안 누른 제안은 만료 — 한 주 전 대화의 제안이 불쑥 일정이 되지 않게.
pub(crate) const PROPOSAL_KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// 제안을 낸 클라이언트 이름 길이 상한(보여 주기만 하는 칸).
const CLIENT_MAX: usize = 60;

/// AI 의 일정 제안 — 팝오버 카드와 MCP 답으로 가는 모양.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Proposal {
    pub id: i64,
    pub client: String,
    pub event: EventInput,
    /// pending · approved · rejected · expired
    pub status: String,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub event_id: Option<i64>,
}

fn row_to_proposal(r: &Row) -> rusqlite::Result<Proposal> {
    Ok(Proposal {
        id: r.get("id")?,
        client: r.get("client")?,
        event: EventInput {
            title: r.get("title")?,
            notes: r.get("notes")?,
            all_day: r.get("all_day")?,
            start_at: r.get("start_at")?,
            end_at: r.get("end_at")?,
            start_date: r.get("start_date")?,
            end_date: r.get("end_date")?,
            alert_min: r.get("alert_min")?,
            private: false,
        },
        status: r.get("status")?,
        created_at: r.get("created_at")?,
        decided_at: r.get("decided_at")?,
        event_id: r.get("event_id")?,
    })
}

const PROPOSAL_COLS: &str = "id, client, title, notes, all_day, start_at, end_at, start_date, end_date, alert_min,
     status, created_at, decided_at, event_id";

/// 일정 한 줄 넣기 — `create_event` 와 `approve_proposal`(트랜잭션 안) 이 같이 쓴다.
fn insert_event(conn: &Connection, v: &Valid, origin: &str, now: i64) -> rusqlite::Result<i64> {
    let (sa, ea, sd, ed) = split_when(&v.when);
    conn.execute(
        "INSERT INTO items (kind, title, notes, all_day, start_at, end_at, start_date, end_date,
                            tz, alert_min, private, origin, created_at, updated_at)
         VALUES ('event', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
        params![
            v.title,
            v.notes,
            matches!(v.when, When::AllDay { .. }),
            sa,
            ea,
            sd,
            ed,
            local_tz(),
            v.alert_min,
            v.private,
            origin,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub(crate) struct Store {
    conn: Mutex<Connection>,
}

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(crate) fn local_tz() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

fn row_to_event(r: &Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: r.get("id")?,
        title: r.get("title")?,
        notes: r.get("notes")?,
        all_day: r.get("all_day")?,
        start_at: r.get("start_at")?,
        end_at: r.get("end_at")?,
        start_date: r.get("start_date")?,
        end_date: r.get("end_date")?,
        alert_min: r.get("alert_min")?,
        private: r.get("private")?,
        origin: r.get("origin")?,
        updated_at: r.get("updated_at")?,
    })
}

const EVENT_COLS: &str =
    "id, title, notes, all_day, start_at, end_at, start_date, end_date, alert_min, private, origin, updated_at";

/// 로컬 시각 창 [from, to) 이 걸치는 달력 날짜 [from_date, to_date) — 종일 일정 겹침 검사용.
fn date_window(from_ms: i64, to_ms: i64) -> (String, String) {
    let day = |ms: i64| {
        Local
            .timestamp_millis_opt(ms)
            .earliest()
            .map(|d| d.date_naive())
            .unwrap_or_default()
    };
    let from = day(from_ms);
    // 끝이 배타라 1ms 앞의 날짜까지가 창 안이다. 창이 비었으면(to ≤ from) 날짜 창도 비운다.
    let to = if to_ms > from_ms {
        day(to_ms - 1).checked_add_days(Days::new(1)).unwrap_or(NaiveDate::MAX)
    } else {
        from
    };
    (from.format("%Y-%m-%d").to_string(), to.format("%Y-%m-%d").to_string())
}

impl Store {
    /// `dir` 안의 `ouro.db` 를 연다(없으면 만든다). 폴더는 0700, 파일은 0600 — 일정은 남이 볼 게 아니다.
    pub(crate) fn open(dir: &Path) -> Result<Self, String> {
        create_private_dir(dir)?;
        let path = dir.join("ouro.db");
        let conn = Connection::open(&path).map_err(|e| format!("DB 열기 실패: {e}"))?;
        restrict_file(&path)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Self {
        Self::init(Connection::open_in_memory().unwrap()).unwrap()
    }

    fn init(conn: Connection) -> Result<Self, String> {
        // WAL = 백업(VACUUM INTO)·읽기가 쓰기를 막지 않는다. foreign_keys 는 연결마다 켜야 한다(SQLite 기본 꺼짐).
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 3000;",
        )
        .map_err(|e| format!("DB 설정 실패: {e}"))?;
        migrate(&conn)?;
        let store = Store { conn: Mutex::new(conn) };
        store.purge_trash(now_ms()).map_err(|e| format!("휴지통 비우기 실패: {e}"))?;
        Ok(store)
    }

    pub(crate) fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // 다른 스레드가 쥔 채 패닉해도 연결 자체는 멀쩡하다(SQLite 가 트랜잭션을 되돌린다) — 독을 무시한다.
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 창 [from_ms, to_ms) 에 걸치는 일정. 시각 일정은 시각으로, 종일 일정은 그 창이 걸치는 로컬 날짜로 겹침을 본다.
    /// 정렬은 대강만 한다(종일 먼저, 그다음 시작순) — 날짜별로 나누고 줄 세우는 건 프론트(`eventsOn`)가 한다.
    pub(crate) fn list_events(&self, from_ms: i64, to_ms: i64) -> Result<Vec<Event>, String> {
        let (from_d, to_d) = date_window(from_ms, to_ms);
        let conn = self.conn();
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT {EVENT_COLS} FROM items
                 WHERE kind = 'event' AND deleted_at IS NULL AND (
                       (all_day = 0 AND start_at < ?2 AND MAX(end_at, start_at + 1) > ?1)
                    OR (all_day = 1 AND start_date < ?4 AND end_date > ?3))
                 ORDER BY COALESCE(start_at, 0), all_day DESC, start_date, id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![from_ms, to_ms, from_d, to_d], row_to_event)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub(crate) fn get_event(&self, id: i64) -> Result<Option<Event>, String> {
        self.conn()
            .query_row(
                &format!(
                    "SELECT {EVENT_COLS} FROM items WHERE id = ?1 AND kind = 'event' AND deleted_at IS NULL"
                ),
                [id],
                row_to_event,
            )
            .optional()
            .map_err(|e| e.to_string())
    }

    pub(crate) fn create_event(&self, input: &EventInput) -> Result<Event, String> {
        let v = validate(input)?;
        let id = insert_event(&self.conn(), &v, "user", now_ms()).map_err(|e| e.to_string())?;
        self.get_event(id)?.ok_or_else(|| "방금 만든 일정을 못 찾았어요".into())
    }

    /// AI 에게 보여도 되는 일정 — `list_events` 에서 비공개만 뺀다. MCP 가 일정을 읽는 **유일한** 길이다.
    pub(crate) fn list_events_for_ai(&self, from_ms: i64, to_ms: i64) -> Result<Vec<Event>, String> {
        Ok(self.list_events(from_ms, to_ms)?.into_iter().filter(|e| !e.private).collect())
    }

    /// 빈 시간 계산용 — [from, to) 에 걸치는 **시각 일정**의 (시작, 끝). 비공개도 바쁨으로 센다(무엇인지는 안 나가지만 언제인지는 짐작된다 — 머리 주석).
    /// 종일 일정은 시간을 막지 않는다(생일·휴가 표시를 «하루 종일 바쁨» 으로 읽으면 빈 시간이 늘 0 이 된다).
    pub(crate) fn busy_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<(i64, i64)>, String> {
        Ok(self
            .list_events(from_ms, to_ms)?
            .into_iter()
            .filter_map(|e| Some((e.start_at?, e.end_at?)))
            .filter(|(s, e)| e > s)
            .collect())
    }

    /// AI 의 제안을 받아 둔다. 일정이 되진 않는다 — 사람이 `approve_proposal` 로 받아야 한다.
    /// 검사는 일정과 같은 `validate`(AI 가 바로 틀린 걸 알 수 있게). 비공개 칸은 받지 않는다(사람이 정할 일).
    pub(crate) fn add_proposal(&self, client: &str, input: &EventInput) -> Result<Proposal, String> {
        let mut input = input.clone();
        input.private = false;
        let v = validate(&input)?;
        let (sa, ea, sd, ed) = split_when(&v.when);
        let client = client_name(client);
        let now = now_ms();
        let id = {
            let conn = self.conn();
            expire_proposals(&conn, now).map_err(|e| e.to_string())?;
            let pending: i64 = conn
                .query_row("SELECT COUNT(*) FROM proposals WHERE status = 'pending'", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if pending >= PENDING_MAX {
                return Err(format!(
                    "기다리는 제안이 이미 {PENDING_MAX}개예요 — 사람이 팝오버에서 정리한 뒤에 다시 내 주세요"
                ));
            }
            conn.execute(
                "INSERT INTO proposals (client, title, notes, all_day, start_at, end_at, start_date, end_date,
                                        alert_min, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![client, v.title, v.notes, input.all_day, sa, ea, sd, ed, v.alert_min, now],
            )
            .map_err(|e| e.to_string())?;
            conn.last_insert_rowid()
        };
        self.get_proposal(id)?.ok_or_else(|| "방금 받은 제안을 못 찾았어요".into())
    }

    pub(crate) fn get_proposal(&self, id: i64) -> Result<Option<Proposal>, String> {
        let conn = self.conn();
        expire_proposals(&conn, now_ms()).map_err(|e| e.to_string())?;
        conn.query_row(&format!("SELECT {PROPOSAL_COLS} FROM proposals WHERE id = ?1"), [id], row_to_proposal)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// 기다리는 제안, 먼저 온 순.
    pub(crate) fn pending_proposals(&self) -> Result<Vec<Proposal>, String> {
        let conn = self.conn();
        expire_proposals(&conn, now_ms()).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT {PROPOSAL_COLS} FROM proposals WHERE status = 'pending' ORDER BY created_at, id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_proposal)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 사람이 제안을 받는다 → 일정 하나. `edited` = 시트에서 고친 값(없으면 제안 그대로).
    /// 상태 확인·일정 넣기·상태 바꾸기가 **한 트랜잭션** — 두 번 눌러도 일정은 하나다.
    pub(crate) fn approve_proposal(&self, id: i64, edited: Option<&EventInput>) -> Result<Event, String> {
        let now = now_ms();
        let event_id = {
            let mut conn = self.conn();
            expire_proposals(&conn, now).map_err(|e| e.to_string())?;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let p = tx
                .query_row(&format!("SELECT {PROPOSAL_COLS} FROM proposals WHERE id = ?1"), [id], row_to_proposal)
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or("없는 제안이에요")?;
            if p.status != "pending" {
                return Err(proposal_gone(&p.status).into());
            }
            let v = validate(edited.unwrap_or(&p.event))?;
            let event_id = insert_event(&tx, &v, "mcp", now).map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE proposals SET status = 'approved', decided_at = ?2, event_id = ?3 WHERE id = ?1",
                params![id, now, event_id],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            event_id
        };
        self.get_event(event_id)?.ok_or_else(|| "받은 일정을 못 찾았어요".into())
    }

    pub(crate) fn reject_proposal(&self, id: i64) -> Result<(), String> {
        let conn = self.conn();
        let n = conn
            .execute(
                "UPDATE proposals SET status = 'rejected', decided_at = ?2 WHERE id = ?1 AND status = 'pending'",
                params![id, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("이미 정리된 제안이에요".into());
        }
        Ok(())
    }

    pub(crate) fn update_event(&self, id: i64, input: &EventInput) -> Result<Event, String> {
        let v = validate(input)?;
        let (sa, ea, sd, ed) = split_when(&v.when);
        let n = self
            .conn()
            .execute(
                "UPDATE items SET title = ?2, notes = ?3, all_day = ?4, start_at = ?5, end_at = ?6,
                                  start_date = ?7, end_date = ?8, alert_min = ?9, private = ?10, updated_at = ?11
                 WHERE id = ?1 AND kind = 'event' AND deleted_at IS NULL",
                params![id, v.title, v.notes, input.all_day, sa, ea, sd, ed, v.alert_min, v.private, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("이미 지워진 일정이에요".into());
        }
        self.get_event(id)?.ok_or_else(|| "고친 일정을 못 찾았어요".into())
    }

    /// 휴지통으로. 되돌리기(`restore_event`) 가 되살린다.
    pub(crate) fn delete_event(&self, id: i64) -> Result<(), String> {
        let n = self
            .conn()
            .execute(
                "UPDATE items SET deleted_at = ?2 WHERE id = ?1 AND kind = 'event' AND deleted_at IS NULL",
                params![id, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("이미 지워진 일정이에요".into());
        }
        Ok(())
    }

    pub(crate) fn restore_event(&self, id: i64) -> Result<Event, String> {
        let n = self
            .conn()
            .execute(
                "UPDATE items SET deleted_at = NULL WHERE id = ?1 AND kind = 'event' AND deleted_at IS NOT NULL",
                [id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("되살릴 일정이 없어요".into());
        }
        self.get_event(id)?.ok_or_else(|| "되살린 일정을 못 찾았어요".into())
    }

    /// 휴지통에서 7일 지난 것을 지운다(부탁·실행·알림 기록은 CASCADE 로 같이).
    fn purge_trash(&self, now: i64) -> rusqlite::Result<usize> {
        self.conn().execute(
            "DELETE FROM items WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
            [now - TRASH_KEEP_MS],
        )
    }

    /// 알림이 걸린 일정 중 [from_ms, to_ms) 근처에 시작하는 것. 울릴 시각 계산은 `alerts` 가 한다.
    /// 알림은 최대 하루(1440분) 전이라 창을 앞으로 하루 더 넓혀서 가져온다.
    pub(crate) fn alert_candidates(&self, from_ms: i64, to_ms: i64) -> Result<Vec<Event>, String> {
        const DAY: i64 = 24 * 60 * 60 * 1000;
        let (from_d, to_d) = date_window(from_ms, to_ms + DAY + 1);
        let conn = self.conn();
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT {EVENT_COLS} FROM items
                 WHERE kind = 'event' AND deleted_at IS NULL AND alert_min IS NOT NULL AND (
                       (all_day = 0 AND start_at >= ?1 AND start_at < ?2)
                    OR (all_day = 1 AND start_date >= ?3 AND start_date < ?4))"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![from_ms, to_ms + DAY + 1, from_d, to_d], row_to_event)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// (일정, 울릴 시각) 을 «띄움» 으로 적는다. 이미 적혀 있으면 false — 두 번 띄우지 않는다.
    pub(crate) fn mark_alert_sent(&self, item_id: i64, fire_at: i64) -> Result<bool, String> {
        let conn = self.conn();
        let n = conn
            .execute(
                "INSERT OR IGNORE INTO alerts_sent (item_id, fire_at, sent_at) VALUES (?1, ?2, ?3)",
                params![item_id, fire_at, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        // 오래된 기록은 쓸모없다(울릴 창이 10분이다). 쌓이지 않게 같이 치운다.
        let _ = conn.execute(
            "DELETE FROM alerts_sent WHERE fire_at < ?1",
            [now_ms() - 30 * 24 * 60 * 60 * 1000],
        );
        Ok(n == 1)
    }

    /// 띄우기에 실패한 알림의 «띄움» 기록을 지운다 — 늦은 알림 창(10분) 안에서 다음 바퀴가 다시 시도한다.
    pub(crate) fn unmark_alert_sent(&self, item_id: i64, fire_at: i64) {
        let _ = self.conn().execute(
            "DELETE FROM alerts_sent WHERE item_id = ?1 AND fire_at = ?2",
            params![item_id, fire_at],
        );
    }

    /// 빠른 입력이 못 알아들은 것을 하나 센다(유형만). 종류는 `parse::Miss` 가 정한다.
    pub(crate) fn record_parse_miss(&self, kind: &str) -> Result<(), String> {
        self.conn()
            .execute(
                "INSERT INTO parse_misses (kind, count, last_at) VALUES (?1, 1, ?2)
                 ON CONFLICT (kind) DO UPDATE SET count = count + 1, last_at = excluded.last_at",
                params![kind, now_ms()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// `dest` 로 스냅숏. VACUUM INTO 는 WAL 에 걸린 쓰기까지 담은 **한 파일짜리 온전한 DB** 를 만든다
    /// (DB 파일만 복사하면 -wal 에 남은 최근 쓰기가 빠진다).
    pub(crate) fn snapshot_to(&self, dest: &Path) -> Result<(), String> {
        let dest = dest.to_str().ok_or("백업 경로가 UTF-8 이 아니에요")?;
        self.conn()
            .execute("VACUUM INTO ?1", [dest])
            .map(|_| ())
            .map_err(|e| format!("백업 실패: {e}"))
    }
}

fn split_when(w: &When) -> (Option<i64>, Option<i64>, Option<String>, Option<String>) {
    match w {
        When::Timed { start, end } => (Some(*start), Some(*end), None, None),
        When::AllDay { start, end } => (
            None,
            None,
            Some(start.format("%Y-%m-%d").to_string()),
            Some(end.format("%Y-%m-%d").to_string()),
        ),
    }
}

/// 제안을 낸 클라이언트가 밝힌 이름을 보여 줄 모양으로 — 제어 문자 빼고 60자(일정·부탁 제안이 같이 쓴다).
pub(crate) fn client_name(c: &str) -> String {
    match c.trim() {
        "" => "unknown".into(),
        c => c.chars().filter(|c| !c.is_control()).take(CLIENT_MAX).collect(),
    }
}

pub(crate) fn proposal_gone(status: &str) -> &'static str {
    match status {
        "approved" => "이미 넣은 제안이에요",
        "rejected" => "이미 거절한 제안이에요",
        _ => "기한이 지난 제안이에요",
    }
}

fn expire_proposals(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE proposals SET status = 'expired', decided_at = ?1 WHERE status = 'pending' AND created_at < ?2",
        params![now, now - PROPOSAL_KEEP_MS],
    )
}

fn migrate(conn: &Connection) -> Result<(), String> {
    let applied: usize = conn
        .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())? as usize;
    if applied > MIGRATIONS.len() {
        // 더 새 버전의 앱이 만진 DB — 모르는 칸을 망가뜨리느니 열지 않는다.
        return Err(format!(
            "이 DB 는 더 새 버전의 Ouro 가 만들었어요(스키마 {applied}, 이 앱은 {})",
            MIGRATIONS.len()
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        // 항목 하나 = 트랜잭션 하나. 중간에 죽어도 반쯤 바뀐 스키마가 남지 않는다.
        let batch = format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1);
        if let Err(e) = conn.execute_batch(&batch) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(format!("스키마 {} 적용 실패: {e}", i + 1));
        }
    }
    Ok(())
}

pub(crate) fn create_private_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{} 만들기 실패: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 못 잠그면 열지 않는다 — «남이 못 읽는다» 를 보장 못 한 채 일정을 쓰게 두지 않는다(코덱스 개발 2).
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("{} 권한 설정 실패: {e}", dir.display()))?;
    }
    Ok(())
}

pub(crate) fn restrict_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{} 권한 설정 실패: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(title: &str, start: i64, end: i64) -> EventInput {
        EventInput {
            title: title.into(),
            notes: String::new(),
            all_day: false,
            start_at: Some(start),
            end_at: Some(end),
            start_date: None,
            end_date: None,
            alert_min: None,
            private: false,
        }
    }

    fn all_day(title: &str, start: &str, end: &str) -> EventInput {
        EventInput {
            title: title.into(),
            notes: String::new(),
            all_day: true,
            start_at: None,
            end_at: None,
            start_date: Some(start.into()),
            end_date: Some(end.into()),
            alert_min: None,
            private: false,
        }
    }

    fn local_ms(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).earliest().unwrap().timestamp_millis()
    }

    #[test]
    fn validate_rejects_bad_shapes() {
        assert!(validate(&timed("  ", 0, 1)).is_err());
        assert!(validate(&timed("a", 10, 5)).is_err());
        assert!(validate(&timed(&"가".repeat(201), 0, 1)).is_err());
        assert!(validate(&all_day("a", "2026-09-28", "2026-09-28")).is_err(), "끝은 배타라 같은 날은 0일");
        assert!(validate(&all_day("a", "2026-9-28", "2026-09-29")).is_err(), "자릿수 고정");
        let mut mixed = timed("a", 0, 1);
        mixed.start_date = Some("2026-09-28".into());
        assert!(validate(&mixed).is_err());
        let mut alert = timed("a", 0, 1);
        alert.alert_min = Some(7);
        assert!(validate(&alert).is_err());
    }

    #[test]
    fn validate_trims_title_and_allows_zero_length() {
        let v = validate(&timed("  치과  ", 5, 5)).unwrap();
        assert_eq!(v.title, "치과");
    }

    #[test]
    fn crud_round_trip() {
        let s = Store::open_in_memory();
        let e = s.create_event(&timed("회의", 1_000, 2_000)).unwrap();
        assert_eq!(e.title, "회의");
        let mut input = timed("회의 (옮김)", 3_000, 4_000);
        input.alert_min = Some(10);
        let e2 = s.update_event(e.id, &input).unwrap();
        assert_eq!((e2.start_at, e2.alert_min), (Some(3_000), Some(10)));
        // 종일로 바꾸면 시각 칸은 비워진다(CHECK 제약이 섞임을 막는다).
        let e3 = s.update_event(e.id, &all_day("쉬는 날", "2026-09-28", "2026-09-29")).unwrap();
        assert_eq!((e3.start_at, e3.start_date.as_deref()), (None, Some("2026-09-28")));

        s.delete_event(e.id).unwrap();
        assert!(s.get_event(e.id).unwrap().is_none());
        assert!(s.delete_event(e.id).is_err(), "두 번 지우기");
        assert!(s.update_event(e.id, &timed("x", 0, 1)).is_err(), "휴지통 안의 것은 못 고친다");
        s.restore_event(e.id).unwrap();
        assert!(s.get_event(e.id).unwrap().is_some());
    }

    #[test]
    fn list_finds_overlaps_only() {
        let s = Store::open_in_memory();
        let day = (local_ms(2026, 9, 28, 0, 0), local_ms(2026, 9, 29, 0, 0));
        let inside = s.create_event(&timed("안", local_ms(2026, 9, 28, 9, 0), local_ms(2026, 9, 28, 10, 0))).unwrap();
        let spans = s.create_event(&timed("밤샘", local_ms(2026, 9, 27, 22, 0), local_ms(2026, 9, 28, 2, 0))).unwrap();
        let zero = s.create_event(&timed("순간", day.0, day.0)).unwrap();
        s.create_event(&timed("어제", local_ms(2026, 9, 27, 9, 0), local_ms(2026, 9, 27, 10, 0))).unwrap();
        s.create_event(&timed("딱 끝에", day.1, day.1 + 1)).unwrap();
        let ad = s.create_event(&all_day("종일", "2026-09-28", "2026-09-29")).unwrap();
        let ad3 = s.create_event(&all_day("여행", "2026-09-26", "2026-09-29")).unwrap();
        s.create_event(&all_day("내일", "2026-09-29", "2026-09-30")).unwrap();

        let ids: Vec<i64> = s.list_events(day.0, day.1).unwrap().iter().map(|e| e.id).collect();
        // 종일이 먼저(COALESCE(start_at,0)=0), 그다음 시작 시각순.
        assert_eq!(ids, vec![ad3.id, ad.id, spans.id, zero.id, inside.id]);
    }

    #[test]
    fn list_skips_trash() {
        let s = Store::open_in_memory();
        let e = s.create_event(&timed("a", 10, 20)).unwrap();
        s.delete_event(e.id).unwrap();
        assert!(s.list_events(0, 100).unwrap().is_empty());
    }

    #[test]
    fn purge_removes_only_old_trash() {
        let s = Store::open_in_memory();
        let old = s.create_event(&timed("old", 0, 1)).unwrap();
        let fresh = s.create_event(&timed("fresh", 0, 1)).unwrap();
        s.delete_event(old.id).unwrap();
        s.delete_event(fresh.id).unwrap();
        s.conn().execute("UPDATE items SET deleted_at = 0 WHERE id = ?1", [old.id]).unwrap();
        assert_eq!(s.purge_trash(now_ms()).unwrap(), 1);
        assert!(s.restore_event(old.id).is_err());
        assert!(s.restore_event(fresh.id).is_ok());
    }

    #[test]
    fn alert_sent_is_once_per_fire_time() {
        let s = Store::open_in_memory();
        let e = s.create_event(&timed("a", 10, 20)).unwrap();
        assert!(s.mark_alert_sent(e.id, now_ms()).unwrap());
        let t = now_ms() + 5;
        assert!(s.mark_alert_sent(e.id, t).unwrap());
        assert!(!s.mark_alert_sent(e.id, t).unwrap());
        s.unmark_alert_sent(e.id, t);
        assert!(s.mark_alert_sent(e.id, t).unwrap(), "실패로 되돌린 알림은 다시 걸린다");
    }

    #[test]
    fn migrations_are_idempotent_and_refuse_newer() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        s.create_event(&timed("남는다", 0, 1)).unwrap();
        drop(s);
        let s = Store::open(dir.path()).unwrap();
        assert_eq!(s.list_events(0, 10).unwrap().len(), 1, "다시 열어도 남는다");
        s.conn().execute_batch("PRAGMA user_version = 99").unwrap();
        drop(s);
        assert!(Store::open(dir.path()).is_err());
    }

    #[test]
    fn upgrades_schema_1_db_keeping_events() {
        // 개발 2 사용자의 DB(스키마 1)가 개발 3 앱에서 열리는 길.
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("ouro.db")).unwrap();
        conn.execute_batch(&format!("{}; PRAGMA user_version = 1;", MIGRATIONS[0])).unwrap();
        conn.execute(
            "INSERT INTO items (kind, title, all_day, start_at, end_at, tz, created_at, updated_at)
             VALUES ('event', '옛 일정', 0, 0, 1, 'Asia/Seoul', 0, 0)",
            [],
        )
        .unwrap();
        drop(conn);
        let s = Store::open(dir.path()).unwrap();
        let v: i64 = s.conn().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
        assert_eq!(s.list_events(0, 10).unwrap()[0].title, "옛 일정");
        s.record_parse_miss("no_date").unwrap();
    }

    #[test]
    fn parse_misses_count_by_kind() {
        let s = Store::open_in_memory();
        s.record_parse_miss("no_date").unwrap();
        s.record_parse_miss("no_date").unwrap();
        s.record_parse_miss("repeat").unwrap();
        let n: i64 = s
            .conn()
            .query_row("SELECT count FROM parse_misses WHERE kind = 'no_date'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn snapshot_is_a_complete_db() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(dir.path()).unwrap();
        s.create_event(&timed("백업될 것", 0, 1)).unwrap();
        let dest = dir.path().join("snap.db");
        s.snapshot_to(&dest).unwrap();
        let copy = Connection::open(&dest).unwrap();
        let n: i64 = copy.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn ai_never_sees_private_but_free_time_counts_it() {
        let s = Store::open_in_memory();
        let mut secret = timed("병원", 1_000, 2_000);
        secret.private = true;
        let secret = s.create_event(&secret).unwrap();
        let open = s.create_event(&timed("회의", 3_000, 4_000)).unwrap();
        let ids: Vec<i64> = s.list_events_for_ai(0, 10_000).unwrap().iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![open.id]);
        assert!(s.list_events(0, 10_000).unwrap().iter().any(|e| e.id == secret.id), "팝오버엔 보인다");
        assert_eq!(s.busy_between(0, 10_000).unwrap(), vec![(1_000, 2_000), (3_000, 4_000)]);
        // 비공개를 고쳐도 비공개가 남는다.
        let mut edit = timed("병원 (옮김)", 1_500, 2_500);
        edit.private = true;
        assert!(s.update_event(secret.id, &edit).unwrap().private);
    }

    #[test]
    fn proposal_becomes_event_only_through_approval() {
        let s = Store::open_in_memory();
        let p = s.add_proposal("claude-code", &timed("치과", 1_000, 2_000)).unwrap();
        assert_eq!(p.status, "pending");
        assert!(s.list_events(0, 10_000).unwrap().is_empty(), "제안만으로는 일정이 없다");
        let e = s.approve_proposal(p.id, None).unwrap();
        assert_eq!((e.title.as_str(), e.origin.as_str()), ("치과", "mcp"));
        let p = s.get_proposal(p.id).unwrap().unwrap();
        assert_eq!((p.status.as_str(), p.event_id), ("approved", Some(e.id)));
        assert!(s.approve_proposal(p.id, None).is_err(), "두 번 눌러도 일정은 하나");
        assert!(s.reject_proposal(p.id).is_err());
        assert_eq!(s.list_events(0, 10_000).unwrap().len(), 1);
    }

    #[test]
    fn proposal_edits_rejects_and_checks() {
        let s = Store::open_in_memory();
        // 모양이 틀린 제안은 받지 않는다(AI 가 바로 안다).
        assert!(s.add_proposal("x", &timed("거꾸로", 5, 1)).is_err());
        // AI 가 비공개로 내도 비공개는 사람이 정한다.
        let mut sneaky = timed("a", 0, 1);
        sneaky.private = true;
        assert!(!s.add_proposal("x", &sneaky).unwrap().event.private);
        // 고쳐서 받기 — 고친 값이 들어가고, 고친 값도 검사한다.
        let p = s.add_proposal("\u{1b}[31mclaude\n", &timed("치과", 1_000, 2_000)).unwrap();
        assert_eq!(p.client, "[31mclaude", "제어 문자는 걷는다");
        assert!(s.approve_proposal(p.id, Some(&timed("", 0, 1))).is_err());
        assert_eq!(s.get_proposal(p.id).unwrap().unwrap().status, "pending", "검사에 걸리면 그대로 기다린다");
        let e = s.approve_proposal(p.id, Some(&timed("치과 (옮김)", 3_000, 4_000))).unwrap();
        assert_eq!(e.start_at, Some(3_000));
        // 거절.
        let q = s.add_proposal("x", &timed("b", 0, 1)).unwrap();
        s.reject_proposal(q.id).unwrap();
        assert!(s.approve_proposal(q.id, None).is_err());
        let left: Vec<String> = s.pending_proposals().unwrap().into_iter().map(|p| p.event.title).collect();
        assert_eq!(left, vec!["a"], "받은 것·거절한 것은 기다리는 목록에서 빠진다");
    }

    #[test]
    fn proposals_cap_and_expire() {
        let s = Store::open_in_memory();
        for i in 0..PENDING_MAX {
            s.add_proposal("x", &timed(&format!("p{i}"), 0, 1)).unwrap();
        }
        assert!(s.add_proposal("x", &timed("넘침", 0, 1)).is_err());
        s.conn().execute("UPDATE proposals SET created_at = 0 WHERE title = 'p0'", []).unwrap();
        assert_eq!(s.pending_proposals().unwrap().len() as i64, PENDING_MAX - 1, "한 주 지난 제안은 만료");
        let old = s.get_proposal(1).unwrap().unwrap();
        assert_eq!(old.status, "expired");
        assert!(s.approve_proposal(old.id, None).is_err());
        s.add_proposal("x", &timed("자리가 났다", 0, 1)).unwrap();
    }

    #[test]
    fn date_window_covers_touched_days() {
        let (f, t) = date_window(local_ms(2026, 9, 28, 0, 0), local_ms(2026, 9, 29, 0, 0));
        assert_eq!((f.as_str(), t.as_str()), ("2026-09-28", "2026-09-29"));
        let (f, t) = date_window(local_ms(2026, 9, 28, 23, 0), local_ms(2026, 9, 29, 1, 0));
        assert_eq!((f.as_str(), t.as_str()), ("2026-09-28", "2026-09-30"));
    }
}
