// 부탁 — «언제 · 누구에게 · 무엇을» 과 그 실행 기록(`runs`). 저장소(`Store`)의 부탁 쪽 문이다.
//
// 설계 결정:
//   · 부탁은 `items`(kind = 'errand') 한 줄 + `errands` 한 줄. 시각은 일정과 같은 `start_at` 이고 길이는 0 이다.
//   · **바깥으로 나가는 문장은 `prompt` 하나**다. 사람이 쓰거나 사람이 승인한 것만 — AI 의 답(`runs.response`)은
//     어디서도 `prompt` 가 되지 않는다(CLAUDE.md 불변 규칙). 지금 부탁을 만드는 길은 팝오버뿐이라 `approved_at` 은 만들 때 찍힌다.
//     AI 가 낸 부탁(개발 6)은 NULL 로 들어와 사람이 승인하기 전엔 `due_errands` 에 안 잡힌다.
//   · **AI 가 낸 부탁은 «제안»**(`errand_proposals`)으로만 들어온다. 부탁이 되는 길은 사람이 누르는 `approve_errand_proposal` 하나 —
//     그때 `approved_at` 이 찍힌다. 고쳐서 승인하면 **사람이 본 글**이 저장된다(AI 가 보낸 글이 아니라). 깊이(`depth`) = 어느 실행의 답에서
//     이어졌나(PLAN §9-4): 사람이 만든 부탁 0 → 그 답에서 이어진 제안 1 → … 최대 3. `from_run_id` 를 밝히지 않은 제안은 0 이다 — 정직한 AI 를
//     위한 안전띠고, 진짜 문은 승인이다(모든 제안은 사람이 읽고 누른다).
//   · 돌았는지는 `runs` 가 정한다. 부탁에 실행이 하나라도 있으면 예약 시각이 와도 다시 안 돈다(건너뜀도 한 줄 남긴다).
//     «다시 실행» 은 예약과 무관한 수동 실행이다(`dispatch::Dispatcher::run_now`).
//   · 한 부탁의 «지금 상태» 는 가장 최근 실행이다. 고치기·지우기는 상태가 «대기» 일 때만(돈 기록을 바꾸지 않는다).
//   · **대상**(개발 7): `claude`(Claude Code) · `codex`(Codex). 부탁 한 줄이 어디로 갈지만 다르고 규칙은 같다.
//   · **반복**(개발 7): `items.rrule` 이 붙은 줄이 «다음 회차» 다. 그 회차가 처음 돌기 시작하는 순간(`claim_run`, 같은 트랜잭션)
//     그다음 회차를 새 줄로 만들고 규칙을 넘긴다 — 회차마다 자기 답을 가진 채 캘린더에 남고, 미래 회차를 미리 깔지 않는다.
//     맥이 사흘 잤으면 놓친 회차는 하나만(그 회차의 «놓치면» 대로) 처리하고 다음 회차는 «지금 이후» 로 잡는다 — 몰아서 세 번 돌지 않는다.
//     반복을 멈추려면 «다음 회차» 를 지운다. 규칙은 고정된 몇 개(`REPEAT_CHOICES`)뿐 — 날짜 계산은 `next_occurrence` 가 결정적으로 한다.
//   · **이어서**(개발 7): 같은 대화를 잇는다(`claude --resume` · `codex exec resume`). 두 길 — 답 시트의 «이어서 부탁»(`resume_run_id`),
//     반복 부탁의 «지난 대화 이어서»(`carry`: 같은 묶음에서 가장 최근 답의 대화). **나가는 글은 여전히 사람이 쓴 `prompt` 하나**다 —
//     앞 대화는 이미 그 AI 쪽에 있는 것이고, 앱이 답을 다시 문장에 붙이지 않는다. 이을 대화는 실행을 확보할 때 정한다(그 사이에 답이 오니까).

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::store::{client_name, local_tz, now_ms, proposal_gone, Store, PROPOSAL_KEEP_MS};

const PROMPT_MAX: usize = 4_000;
const TITLE_CHARS: usize = 40;
/// 받아 둘 답의 길이(글자). 거대한 답이 DB 와 팝오버를 키우지 않게.
const RESPONSE_MAX: usize = 100_000;
const STDERR_MAX: usize = 2_000;
/// 부탁에 줄 수 있는 도구 — 빈 값 = 대화만. 더 늘릴 땐 이 목록에 한 줄(PLAN §9-2: 카드에서 명시한 것만).
pub(crate) const TOOL_CHOICES: [&str; 2] = ["", "WebSearch"];
/// 부탁을 받을 수 있는 쪽. 스키마 1 의 CHECK 와 같다.
pub(crate) const TARGETS: [&str; 2] = ["claude", "codex"];
/// 고를 수 있는 반복 — RFC 5545 RRULE 글자 그대로 저장한다(나중에 `.ics` 로 그대로 나간다). 빈 값 = 반복 안 함.
/// 매월은 뺐다 — 31일에 건 «매월» 이 2월에 무엇이 되는지 사람마다 기대가 달라서, 고를 수 있게 하려면 그 질문부터 답해야 한다.
pub(crate) const REPEAT_CHOICES: [&str; 4] = ["", "FREQ=DAILY", "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "FREQ=WEEKLY"];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrandInput {
    pub prompt: String,
    pub start_at: i64,
    #[serde(default)]
    pub allowed_tools: String,
    /// `run` = 놓쳐도 늦게 실행 / `skip` = 건너뜀
    #[serde(default = "default_late")]
    pub late: String,
    /// `claude` · `codex`
    #[serde(default = "default_target")]
    pub target: String,
    /// `REPEAT_CHOICES` 중 하나. 빈 값 = 한 번.
    #[serde(default)]
    pub repeat: String,
    /// 반복할 때 지난 회차의 대화를 잇는다.
    #[serde(default)]
    pub carry: bool,
    /// «이어서 부탁» — 이 실행의 대화를 잇는다. 만들 때만 정해지고 고칠 수 없다.
    #[serde(default)]
    pub resume_run_id: Option<i64>,
}

impl Default for ErrandInput {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            start_at: 0,
            allowed_tools: String::new(),
            late: default_late(),
            target: default_target(),
            repeat: String::new(),
            carry: false,
            resume_run_id: None,
        }
    }
}

fn default_late() -> String {
    "run".into()
}

fn default_target() -> String {
    "claude".into()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunView {
    pub id: i64,
    /// running · done · failed · stopped · skipped
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// 예약보다 얼마나 늦게 시작했나(ms).
    pub late_ms: i64,
    /// 바깥으로 나간 원문(PLAN §9-6).
    pub sent_text: String,
    pub response: Option<String>,
    pub stderr: Option<String>,
    pub read: bool,
    /// 앞 대화를 이어서 보냈나.
    pub resumed: bool,
    /// 이 답에서 «이어서 부탁» 을 할 수 있나(대화 번호를 받았다).
    pub can_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Errand {
    pub id: i64,
    pub title: String,
    pub prompt: String,
    pub target: String,
    pub start_at: i64,
    pub allowed_tools: String,
    pub late: String,
    pub approved: bool,
    /// 이 줄이 «다음 회차» 면 그 규칙(`REPEAT_CHOICES`), 아니면 빈 값.
    pub repeat: String,
    /// 반복 묶음에 속한다(지난 회차 포함).
    pub series: bool,
    pub carry: bool,
    pub resume_run_id: Option<i64>,
    /// 잇는 대화의 원래 부탁 제목 — 시트가 «… 에 이어서» 로 보인다.
    pub resume_title: Option<String>,
    /// 가장 최근 실행. 없으면 «대기».
    pub run: Option<RunView>,
}

/// 돌 차례가 된 부탁. `session`·`folder` 는 `claim_run` 이 정한다(목록에선 비어 있다).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Due {
    pub id: i64,
    pub prompt: String,
    pub allowed_tools: String,
    pub late: String,
    pub start_at: i64,
    pub target: String,
    /// 이을 대화 번호. None = 새 대화.
    pub session: Option<String>,
    /// 작업 폴더 `runs/<folder>`.
    pub folder: i64,
}

/// 실행 권리를 어떤 길로 얻나. 수동은 «요청한 뒤 다른 실행이 시작하지 않았을 때만».
#[derive(Debug, Clone, Copy)]
pub(crate) enum Claim {
    Scheduled,
    Manual { since: i64 },
}

/// 실행이 끝난 모양.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub response: Option<String>,
    pub stderr: Option<String>,
    pub session_id: Option<String>,
}

pub(crate) fn title_of(prompt: &str) -> String {
    let first = prompt.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let t: String = first.chars().take(TITLE_CHARS).collect();
    if first.chars().count() > TITLE_CHARS {
        format!("{}…", t.trim_end())
    } else {
        t
    }
}

fn validate(input: &ErrandInput) -> Result<(String, String), String> {
    let prompt = input.prompt.trim();
    if prompt.is_empty() {
        return Err("부탁할 말을 적어 주세요".into());
    }
    if prompt.chars().count() > PROMPT_MAX {
        return Err(format!("부탁은 {PROMPT_MAX}자까지예요"));
    }
    if !TOOL_CHOICES.contains(&input.allowed_tools.as_str()) {
        return Err("허용할 도구가 이상해요".into());
    }
    if !matches!(input.late.as_str(), "run" | "skip") {
        return Err("놓쳤을 때 할 일이 이상해요".into());
    }
    if !TARGETS.contains(&input.target.as_str()) {
        return Err("부탁 받을 쪽이 이상해요".into());
    }
    if !REPEAT_CHOICES.contains(&input.repeat.as_str()) {
        return Err("반복이 이상해요".into());
    }
    if input.carry && input.repeat.is_empty() {
        return Err("«지난 대화 이어서» 는 반복하는 부탁에만 돼요".into());
    }
    if input.resume_run_id.is_some() && !input.repeat.is_empty() {
        return Err("이어서 하는 부탁은 반복할 수 없어요 — 반복은 새 부탁으로 걸어 주세요".into());
    }
    Ok((prompt.to_string(), title_of(prompt)))
}

/// «이어서 부탁» 이 잇는 실행을 확인한다 — 답이 왔고 대화 번호가 있어야 하고, 같은 쪽(Claude·Codex)이어야 한다.
/// 돌려주는 값 = 그 대화가 사는 작업 폴더(부모 부탁의 폴더).
/// (실행 상태, 대화 번호, 받은 쪽, 부탁 id, 작업 폴더)
type ParentRun = (String, Option<String>, String, i64, Option<i64>);

fn resume_folder(conn: &rusqlite::Connection, run_id: i64, target: &str) -> Result<i64, String> {
    let row: Option<ParentRun> = conn
        .query_row(
            "SELECT r.status, r.session_id, e.target, e.item_id, e.folder_id
             FROM runs r JOIN errands e ON e.item_id = r.item_id WHERE r.id = ?1",
            [run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let (status, session, parent_target, item, folder) = row.ok_or("이을 답이 없어요")?;
    if status != "done" || session.is_none() {
        return Err("답이 온 부탁만 이어서 부탁할 수 있어요".into());
    }
    if parent_target != target {
        return Err("이어서 하는 부탁은 같은 쪽(Claude Code·Codex)에게만 보낼 수 있어요".into());
    }
    Ok(folder.unwrap_or(item))
}

/// 반복 규칙의 다음 회차 (순수 — 테스트 가능): `start_at` 의 **로컬 벽시계 시각**을 지키며 `start_at` 과 `after` 둘 다보다 뒤인 첫 회차.
/// 서머타임으로 그 시각이 두 번이면 앞의 것, 없으면(건너뛴 한 시간) 한 시간 뒤. 400일 안에 없으면 None.
pub(crate) fn next_occurrence(rule: &str, start_at: i64, after: i64) -> Option<i64> {
    use chrono::{Datelike, Duration as CDuration, Local, TimeZone, Weekday};
    let start = Local.timestamp_millis_opt(start_at).earliest()?;
    let time = start.time();
    let after_day = Local.timestamp_millis_opt(after).earliest()?.date_naive();
    let mut day = start.date_naive().succ_opt()?.max(after_day);
    for _ in 0..400 {
        let hit = match rule {
            "FREQ=DAILY" => true,
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" => !matches!(day.weekday(), Weekday::Sat | Weekday::Sun),
            "FREQ=WEEKLY" => day.weekday() == start.weekday(),
            _ => return None,
        };
        if hit {
            let wall = day.and_time(time);
            let at = Local
                .from_local_datetime(&wall)
                .earliest()
                .or_else(|| Local.from_local_datetime(&(wall + CDuration::hours(1))).earliest())?
                .timestamp_millis();
            if at > after && at > start_at {
                return Some(at);
            }
        }
        day = day.succ_opt()?;
    }
    None
}

/// 꼬리 글자 `n` 개만.
fn tail(s: &str, n: usize) -> String {
    let c = s.chars().count();
    if c <= n {
        s.to_string()
    } else {
        format!("…{}", s.chars().skip(c - n).collect::<String>())
    }
}

const LIST_SQL: &str = "
    SELECT i.id, i.title, i.start_at, e.target, e.prompt, e.allowed_tools, e.late, e.approved_at,
           r.id, r.status, r.started_at, r.finished_at, r.late_ms, r.sent_text, r.response, r.stderr, r.read_at,
           r.resumed_session, r.session_id,
           COALESCE(i.rrule, ''), e.series_id, e.carry, e.resume_run_id,
           (SELECT pi.title FROM runs pr JOIN items pi ON pi.id = pr.item_id WHERE pr.id = e.resume_run_id)
    FROM items i
    JOIN errands e ON e.item_id = i.id
    LEFT JOIN runs r ON r.id = (SELECT MAX(id) FROM runs WHERE item_id = i.id)
    WHERE i.kind = 'errand' AND i.deleted_at IS NULL";

fn row_to_errand(r: &rusqlite::Row) -> rusqlite::Result<Errand> {
    let run = match r.get::<_, Option<i64>>(8)? {
        None => None,
        Some(id) => {
            let status: String = r.get(9)?;
            Some(RunView {
                id,
                can_resume: status == "done" && r.get::<_, Option<String>>(18)?.is_some(),
                status,
                started_at: r.get(10)?,
                finished_at: r.get(11)?,
                late_ms: r.get(12)?,
                sent_text: r.get(13)?,
                response: r.get(14)?,
                stderr: r.get(15)?,
                read: r.get::<_, Option<i64>>(16)?.is_some(),
                resumed: r.get::<_, Option<String>>(17)?.is_some(),
            })
        }
    };
    Ok(Errand {
        id: r.get(0)?,
        title: r.get(1)?,
        start_at: r.get(2)?,
        target: r.get(3)?,
        prompt: r.get(4)?,
        allowed_tools: r.get(5)?,
        late: r.get(6)?,
        approved: r.get::<_, Option<i64>>(7)?.is_some(),
        repeat: r.get(19)?,
        series: r.get::<_, Option<i64>>(20)?.is_some(),
        carry: r.get(21)?,
        resume_run_id: r.get(22)?,
        resume_title: r.get(23)?,
        run,
    })
}

/// 부탁 한 줄 넣기 — `create_errand`·`approve_errand_proposal`·다음 회차(`claim_run`)가 같이 쓴다. 트랜잭션 안에서 부른다.
struct NewErrand<'a> {
    title: &'a str,
    prompt: &'a str,
    start_at: i64,
    target: &'a str,
    allowed_tools: &'a str,
    late: &'a str,
    origin: &'a str,
    approved_at: i64,
    rrule: Option<&'a str>,
    series_id: Option<i64>,
    carry: bool,
    resume_run_id: Option<i64>,
    folder_id: Option<i64>,
    parent_run_id: Option<i64>,
    depth: i64,
}

fn insert_errand(tx: &rusqlite::Connection, n: &NewErrand, now: i64) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO items (kind, title, all_day, start_at, end_at, tz, rrule, origin, created_at, updated_at)
         VALUES ('errand', ?1, 0, ?2, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![n.title, n.start_at, local_tz(), n.rrule, n.origin, now],
    )?;
    let id = tx.last_insert_rowid();
    // 반복이면 첫 줄이 곧 묶음 번호다.
    let series = n.series_id.or(n.rrule.map(|_| id));
    tx.execute(
        "INSERT INTO errands (item_id, target, prompt, allowed_tools, late, approved_at, parent_run_id, depth,
                              series_id, carry, resume_run_id, folder_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            n.target,
            n.prompt,
            n.allowed_tools,
            n.late,
            n.approved_at,
            n.parent_run_id,
            n.depth,
            series,
            n.carry,
            n.resume_run_id,
            n.folder_id
        ],
    )?;
    Ok(id)
}

const DUE_COLS: &str = "i.id, e.prompt, e.allowed_tools, e.late, i.start_at, e.target, COALESCE(e.folder_id, i.id)";

fn row_to_due(r: &rusqlite::Row) -> rusqlite::Result<Due> {
    Ok(Due {
        id: r.get(0)?,
        prompt: r.get(1)?,
        allowed_tools: r.get(2)?,
        late: r.get(3)?,
        start_at: r.get(4)?,
        target: r.get(5)?,
        session: None,
        folder: r.get(6)?,
    })
}

impl Store {
    /// 사람이 만든 부탁 — 만드는 순간 승인된 것으로 친다(바깥으로 나갈 문장을 본인이 썼다).
    pub(crate) fn create_errand(&self, input: &ErrandInput) -> Result<Errand, String> {
        let (prompt, title) = validate(input)?;
        let now = now_ms();
        let id = {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let folder_id = input.resume_run_id.map(|r| resume_folder(&tx, r, &input.target)).transpose()?;
            let n = NewErrand {
                title: &title,
                prompt: &prompt,
                start_at: input.start_at,
                target: &input.target,
                allowed_tools: &input.allowed_tools,
                late: &input.late,
                origin: "user",
                approved_at: now,
                rrule: Some(input.repeat.as_str()).filter(|r| !r.is_empty()),
                series_id: None,
                carry: input.carry,
                resume_run_id: input.resume_run_id,
                folder_id,
                parent_run_id: None,
                depth: 0,
            };
            let id = insert_errand(&tx, &n, now).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            id
        };
        self.get_errand(id)?.ok_or_else(|| "방금 만든 부탁을 못 찾았어요".into())
    }

    pub(crate) fn get_errand(&self, id: i64) -> Result<Option<Errand>, String> {
        self.conn()
            .query_row(&format!("{LIST_SQL} AND i.id = ?1"), [id], row_to_errand)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// 예약 시각이 [from_ms, to_ms) 인 부탁.
    pub(crate) fn list_errands(&self, from_ms: i64, to_ms: i64) -> Result<Vec<Errand>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare_cached(&format!("{LIST_SQL} AND i.start_at >= ?1 AND i.start_at < ?2 ORDER BY i.start_at, i.id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![from_ms, to_ms], row_to_errand)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 아직 한 번도 안 돈 부탁만 고친다 — 돈 기록(보낸 원문)이 «지금 문장» 과 어긋나지 않게.
    /// «이어서» 는 만들 때 정해진 대로 둔다(입력의 `resume_run_id` 는 안 본다) — 잇는 대화가 있으면 받는 쪽을 못 바꾼다.
    pub(crate) fn update_errand(&self, id: i64, input: &ErrandInput) -> Result<Errand, String> {
        {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let has_run: bool = tx
                .query_row("SELECT EXISTS (SELECT 1 FROM runs WHERE item_id = ?1)", [id], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if has_run {
                return Err("이미 돈 부탁은 고칠 수 없어요 — 새로 만들어 주세요".into());
            }
            let resume: Option<Option<i64>> = tx
                .query_row("SELECT resume_run_id FROM errands WHERE item_id = ?1", [id], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            let resume = resume.ok_or("이미 지워진 부탁이에요")?;
            let input = ErrandInput { resume_run_id: resume, ..input.clone() };
            let (prompt, title) = validate(&input)?;
            if let Some(r) = resume {
                resume_folder(&tx, r, &input.target)?;
            }
            let rrule = Some(input.repeat.as_str()).filter(|r| !r.is_empty());
            let n = tx
                .execute(
                    "UPDATE items SET title = ?2, start_at = ?3, end_at = ?3, rrule = ?4, updated_at = ?5
                     WHERE id = ?1 AND kind = 'errand' AND deleted_at IS NULL",
                    params![id, title, input.start_at, rrule, now_ms()],
                )
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("이미 지워진 부탁이에요".into());
            }
            // 반복을 새로 걸면 이 줄이 묶음의 첫 줄이 된다(이미 묶음이면 그대로).
            tx.execute(
                "UPDATE errands SET prompt = ?2, allowed_tools = ?3, late = ?4, target = ?5, carry = ?6,
                        series_id = CASE WHEN ?7 THEN COALESCE(series_id, item_id) ELSE series_id END
                 WHERE item_id = ?1",
                params![id, prompt, input.allowed_tools, input.late, input.target, input.carry, rrule.is_some()],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        self.get_errand(id)?.ok_or_else(|| "고친 부탁을 못 찾았어요".into())
    }

    /// 휴지통으로(되돌리기 가능). 도는 중이면 거절 — 먼저 멈춘다.
    pub(crate) fn delete_errand(&self, id: i64) -> Result<(), String> {
        let conn = self.conn();
        let running: bool = conn
            .query_row("SELECT EXISTS (SELECT 1 FROM runs WHERE item_id = ?1 AND status = 'running')", [id], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if running {
            return Err("도는 중이에요 — 먼저 멈춰 주세요".into());
        }
        let n = conn
            .execute(
                "UPDATE items SET deleted_at = ?2 WHERE id = ?1 AND kind = 'errand' AND deleted_at IS NULL",
                params![id, now_ms()],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("이미 지워진 부탁이에요".into());
        }
        Ok(())
    }

    pub(crate) fn restore_errand(&self, id: i64) -> Result<Errand, String> {
        let n = self
            .conn()
            .execute(
                "UPDATE items SET deleted_at = NULL WHERE id = ?1 AND kind = 'errand' AND deleted_at IS NOT NULL",
                [id],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("되살릴 부탁이 없어요".into());
        }
        self.get_errand(id)?.ok_or_else(|| "되살린 부탁을 못 찾았어요".into())
    }

    /// 돌 차례인 부탁 — 승인됐고, 예약 시각이 지났고, 한 번도 안 돈 것. 오래된 순.
    pub(crate) fn due_errands(&self, now: i64) -> Result<Vec<Due>, String> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare_cached(&format!(
                "SELECT {DUE_COLS}
                 FROM items i JOIN errands e ON e.item_id = i.id
                 WHERE i.kind = 'errand' AND i.deleted_at IS NULL AND e.approved_at IS NOT NULL
                   AND i.start_at <= ?1
                   AND NOT EXISTS (SELECT 1 FROM runs WHERE item_id = i.id)
                 ORDER BY i.start_at, i.id"
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([now], row_to_due)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 아직 안 돈 부탁 중 가장 이른 예약 시각(디스패처가 거기까지 잔다).
    pub(crate) fn next_errand_at(&self, after: i64) -> Result<Option<i64>, String> {
        self.conn()
            .query_row(
                "SELECT MIN(i.start_at) FROM items i JOIN errands e ON e.item_id = i.id
                 WHERE i.kind = 'errand' AND i.deleted_at IS NULL AND e.approved_at IS NOT NULL
                   AND i.start_at > ?1 AND NOT EXISTS (SELECT 1 FROM runs WHERE item_id = i.id)",
                [after],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
    }

    /// 실행할 권리를 **한 트랜잭션**으로 확보한다(코덱스 개발 5): 지금 DB 의 부탁을 다시 읽어 — 지워졌거나 승인이 없거나,
    /// 예약 실행인데 이미 돌았으면(또는 수동 요청 뒤에 다른 실행이 시작했으면) `None` — 그 값으로 실행 기록을 연다.
    /// 대기하는 동안 사람이 고치거나 지운 부탁이 옛 문장으로 나가지 않게 하는 문이다. `sent_text` = 지금 문장 그대로.
    /// `skip` 이 이유를 돌려주면 돌리지 않고 «건너뜀» 으로 바로 닫는다. 돌려주는 bool = 건너뜀 여부.
    ///
    /// 개발 7: 이을 대화(`Due::session`)도 여기서 정한다 — 반복의 «지난 대화» 는 바로 앞 회차의 답이 와야 생긴다.
    /// 그리고 **반복의 다음 회차가 처음 돌기 시작하면**(예약·수동·건너뜀 모두) 같은 트랜잭션에서 그다음 회차를 만들고 규칙을 넘긴다.
    pub(crate) fn claim_run(
        &self,
        id: i64,
        claim: Claim,
        skip: impl Fn(&Due, i64) -> Option<&'static str>,
    ) -> Result<Option<(i64, Due, bool)>, String> {
        let now = now_ms();
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let due = tx
            .query_row(
                &format!(
                    "SELECT {DUE_COLS}, i.title, i.rrule, e.series_id, e.carry, e.resume_run_id, e.approved_at,
                            e.parent_run_id, e.depth, i.origin
                     FROM items i JOIN errands e ON e.item_id = i.id
                     WHERE i.id = ?1 AND i.kind = 'errand' AND i.deleted_at IS NULL AND e.approved_at IS NOT NULL"
                ),
                [id],
                |r| {
                    Ok((
                        row_to_due(r)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, Option<String>>(8)?,
                        r.get::<_, Option<i64>>(9)?,
                        r.get::<_, bool>(10)?,
                        r.get::<_, Option<i64>>(11)?,
                        r.get::<_, i64>(12)?,
                        (r.get::<_, Option<i64>>(13)?, r.get::<_, i64>(14)?, r.get::<_, String>(15)?),
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((mut due, title, rrule, series, carry, resume_run, approved_at, (parent_run, depth, origin))) = due else {
            return Ok(None);
        };
        let (running, any_since, any): (bool, bool, bool) = tx
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM runs WHERE item_id = ?1 AND status = 'running'),
                        EXISTS (SELECT 1 FROM runs WHERE item_id = ?1 AND started_at >= ?2 AND status != 'skipped'),
                        EXISTS (SELECT 1 FROM runs WHERE item_id = ?1)",
                params![id, if let Claim::Manual { since } = claim { since } else { i64::MAX }],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| e.to_string())?;
        let late_ms = match claim {
            Claim::Scheduled => {
                if any || due.start_at > now {
                    return Ok(None);
                }
                (now - due.start_at).max(0)
            }
            Claim::Manual { .. } => {
                if running || any_since {
                    return Ok(None);
                }
                0
            }
        };
        let reason = if matches!(claim, Claim::Scheduled) { skip(&due, late_ms) } else { None };
        // 이을 대화: «이어서 부탁» 이면 그 실행의 대화, 반복의 «지난 대화 이어서» 면 같은 묶음·같은 쪽에서 가장 최근 답의 대화.
        due.session = match (resume_run, carry, series) {
            (Some(r), _, _) => tx
                .query_row("SELECT session_id FROM runs WHERE id = ?1", [r], |r| r.get::<_, Option<String>>(0))
                .optional()
                .map_err(|e| e.to_string())?
                .flatten(),
            (None, true, Some(sid)) => tx
                .query_row(
                    "SELECT r.session_id FROM runs r JOIN errands e ON e.item_id = r.item_id
                     WHERE e.series_id = ?1 AND e.target = ?2 AND r.status = 'done' AND r.session_id IS NOT NULL
                     ORDER BY r.id DESC LIMIT 1",
                    params![sid, due.target],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?,
            _ => None,
        };
        if reason.is_some() {
            due.session = None;
        }
        tx.execute(
            "INSERT INTO runs (item_id, started_at, finished_at, sent_text, status, late_ms, stderr, resumed_session)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                now,
                reason.map(|_| now),
                due.prompt,
                if reason.is_some() { "skipped" } else { "running" },
                late_ms,
                reason,
                due.session
            ],
        )
        .map_err(|e| e.to_string())?;
        let run_id = tx.last_insert_rowid();
        // 반복: 이 회차가 처음 도는 순간 다음 회차를 만든다. «지금 이후» 로 잡아 놓친 회차를 몰아 돌리지 않는다.
        if let (Some(rule), false) = (rrule.as_deref(), any) {
            match next_occurrence(rule, due.start_at, now.max(due.start_at)) {
                Some(at) => {
                    let n = NewErrand {
                        title: &title,
                        prompt: &due.prompt,
                        start_at: at,
                        target: &due.target,
                        allowed_tools: &due.allowed_tools,
                        late: &due.late,
                        origin: &origin,
                        approved_at,
                        rrule: Some(rule),
                        series_id: series.or(Some(id)),
                        carry,
                        resume_run_id: None,
                        folder_id: Some(series.unwrap_or(id)),
                        parent_run_id: parent_run,
                        depth,
                    };
                    insert_errand(&tx, &n, now).map_err(|e| e.to_string())?;
                    tx.execute("UPDATE items SET rrule = NULL WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
                }
                None => eprintln!("ouro: 반복의 다음 회차를 못 찾았어요 — {rule}"),
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Some((run_id, due, reason.is_some())))
    }

    /// 테스트용 얇은 문 — 기록 한 줄을 그냥 연다.
    #[cfg(test)]
    pub(crate) fn begin_run(&self, item_id: i64, sent_text: &str, late_ms: i64, skipped: Option<&str>) -> Result<i64, String> {
        let now = now_ms();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO runs (item_id, started_at, finished_at, sent_text, status, late_ms, stderr)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![item_id, now, skipped.map(|_| now), sent_text, if skipped.is_some() { "skipped" } else { "running" }, late_ms.max(0), skipped],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    pub(crate) fn finish_run(&self, run_id: i64, o: &Outcome) -> Result<(), String> {
        // 너무 긴 답은 자르되 «잘렸다» 를 글에 남긴다 — 완전한 답처럼 읽히지 않게(코덱스 개발 5).
        let response = o.response.as_deref().map(|s| {
            if s.chars().count() > RESPONSE_MAX {
                format!("{}\n\n… (너무 길어서 여기까지만 저장했어요)", s.chars().take(RESPONSE_MAX).collect::<String>())
            } else {
                s.to_string()
            }
        });
        let stderr = o.stderr.as_deref().filter(|s| !s.trim().is_empty()).map(|s| tail(s.trim(), STDERR_MAX));
        self.conn()
            .execute(
                "UPDATE runs SET finished_at = ?2, status = ?3, exit_code = ?4, response = ?5, stderr = ?6, session_id = ?7
                 WHERE id = ?1",
                params![run_id, now_ms(), o.status, o.exit_code, response, stderr, o.session_id],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// 앱이 꺼진 채 «도는 중» 으로 남은 기록 — 프로세스는 이미 없다. 실패로 닫는다.
    pub(crate) fn fail_orphan_runs(&self) -> Result<usize, String> {
        self.conn()
            .execute(
                "UPDATE runs SET status = 'failed', finished_at = ?1, stderr = '앱이 꺼지면서 끊겼어요'
                 WHERE status = 'running'",
                [now_ms()],
            )
            .map_err(|e| e.to_string())
    }

    pub(crate) fn mark_run_read(&self, run_id: i64) -> Result<(), String> {
        self.conn()
            .execute("UPDATE runs SET read_at = ?2 WHERE id = ?1 AND read_at IS NULL", params![run_id, now_ms()])
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// 이 부탁이 지금 도는 중인가.
    pub(crate) fn is_running(&self, item_id: i64) -> Result<bool, String> {
        self.conn()
            .query_row("SELECT EXISTS (SELECT 1 FROM runs WHERE item_id = ?1 AND status = 'running')", [item_id], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    /// `since` 이후 실제로 돈(건너뜀 제외) 실행 수 — 하루 상한 계산.
    pub(crate) fn runs_since(&self, since: i64) -> Result<i64, String> {
        self.conn()
            .query_row("SELECT COUNT(*) FROM runs WHERE started_at >= ?1 AND status != 'skipped'", [since], |r| r.get(0))
            .map_err(|e| e.to_string())
    }

    /// 수동 실행 전 확인 — 있고 승인된 부탁인가.
    pub(crate) fn errand_runnable(&self, id: i64) -> Result<bool, String> {
        Ok(self.get_errand(id)?.is_some_and(|e| e.approved))
    }
}

// ── AI 의 부탁 제안 ──────────────────────────────────────────

/// 한꺼번에 기다릴 수 있는 부탁 제안 수. 카드마다 글을 읽어야 해서 일정 제안(20)보다 적게.
const ERRAND_PENDING_MAX: i64 = 10;
/// 답에서 답으로 이어질 수 있는 단계(PLAN §9-4).
pub(crate) const DEPTH_MAX: i64 = 3;
/// 제안 시각이 이보다 먼 미래면 거부(해를 잘못 쓴 제안이 조용히 묻히지 않게).
const FAR_MS: i64 = 366 * 24 * 60 * 60 * 1000;
/// 이만큼 넘게 지난 시각의 제안은 못 받는다 — 받으면 바로 «늦게 실행» 이거나 건너뜀이 된다. 사람이 때를 고쳐서 받는다.
const PAST_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ErrandProposalInput {
    pub errand: ErrandInput,
    /// 어느 실행의 답에서 이어진 부탁인가(깊이 계산용).
    pub from_run_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrandProposal {
    pub id: i64,
    pub client: String,
    pub prompt: String,
    pub start_at: i64,
    pub allowed_tools: String,
    pub late: String,
    pub target: String,
    pub depth: i64,
    pub parent_run_id: Option<i64>,
    /// pending · approved · rejected · expired
    pub status: String,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub errand_id: Option<i64>,
}

const EP_COLS: &str =
    "id, client, prompt, start_at, allowed_tools, late, depth, parent_run_id, status, created_at, decided_at, errand_id, target";

fn row_to_ep(r: &rusqlite::Row) -> rusqlite::Result<ErrandProposal> {
    Ok(ErrandProposal {
        id: r.get(0)?,
        client: r.get(1)?,
        prompt: r.get(2)?,
        start_at: r.get(3)?,
        allowed_tools: r.get(4)?,
        late: r.get(5)?,
        depth: r.get(6)?,
        parent_run_id: r.get(7)?,
        status: r.get(8)?,
        created_at: r.get(9)?,
        decided_at: r.get(10)?,
        errand_id: r.get(11)?,
        target: r.get(12)?,
    })
}

fn expire_errand_proposals(conn: &rusqlite::Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE errand_proposals SET status = 'expired', decided_at = ?1 WHERE status = 'pending' AND created_at < ?2",
        params![now, now - PROPOSAL_KEEP_MS],
    )
}

impl Store {
    /// AI 의 부탁 제안을 받아 둔다. 부탁이 되진 않는다 — 사람이 `approve_errand_proposal` 로 받아야 돈다.
    /// 검사는 부탁과 같은 `validate`. 깊이가 한도를 넘으면 거절(사람이 직접 부탁으로 만들면 된다).
    pub(crate) fn add_errand_proposal(&self, client: &str, input: &ErrandProposalInput) -> Result<ErrandProposal, String> {
        let (prompt, _) = validate(&input.errand)?;
        // AI 는 한 번짜리 부탁만 낸다 — 반복·이어서는 사람이 정한다(고리가 스스로 굴러가지 않게, PLAN §9-4).
        if !input.errand.repeat.is_empty() || input.errand.carry || input.errand.resume_run_id.is_some() {
            return Err("AI 제안은 한 번짜리 부탁만 돼요".into());
        }
        let now = now_ms();
        if input.errand.start_at < now - PAST_MS {
            return Err("시각이 이미 지났어요 — 지금 이후의 때로 제안해 주세요".into());
        }
        if input.errand.start_at > now + FAR_MS {
            return Err("시각이 1년 넘게 멀어요 — 날짜를 다시 확인해 주세요".into());
        }
        let client = client_name(client);
        let id = {
            let conn = self.conn();
            expire_errand_proposals(&conn, now).map_err(|e| e.to_string())?;
            let pending: i64 = conn
                .query_row("SELECT COUNT(*) FROM errand_proposals WHERE status = 'pending'", [], |r| r.get(0))
                .map_err(|e| e.to_string())?;
            if pending >= ERRAND_PENDING_MAX {
                return Err("기다리는 부탁 제안이 너무 많아요 — 사람이 먼저 정리해야 해요".into());
            }
            let depth = match input.from_run_id {
                None => 0,
                Some(run) => {
                    let parent: Option<i64> = conn
                        .query_row(
                            "SELECT e.depth FROM runs r JOIN errands e ON e.item_id = r.item_id WHERE r.id = ?1",
                            [run],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(|e| e.to_string())?;
                    let d = parent.ok_or("from_run_id 가 가리키는 실행이 없어요")? + 1;
                    if d > DEPTH_MAX {
                        return Err(format!(
                            "답에서 답으로 {DEPTH_MAX}번 이어진 부탁이에요 — 더 이어 가려면 사람이 직접 부탁을 만들어야 해요"
                        ));
                    }
                    d
                }
            };
            conn.execute(
                "INSERT INTO errand_proposals (client, prompt, start_at, allowed_tools, late, parent_run_id, depth, created_at, target)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    client,
                    prompt,
                    input.errand.start_at,
                    input.errand.allowed_tools,
                    input.errand.late,
                    input.from_run_id,
                    depth,
                    now,
                    input.errand.target
                ],
            )
            .map_err(|e| e.to_string())?;
            conn.last_insert_rowid()
        };
        self.get_errand_proposal(id)?.ok_or_else(|| "방금 받은 제안을 못 찾았어요".into())
    }

    pub(crate) fn get_errand_proposal(&self, id: i64) -> Result<Option<ErrandProposal>, String> {
        let conn = self.conn();
        expire_errand_proposals(&conn, now_ms()).map_err(|e| e.to_string())?;
        conn.query_row(&format!("SELECT {EP_COLS} FROM errand_proposals WHERE id = ?1"), [id], row_to_ep)
            .optional()
            .map_err(|e| e.to_string())
    }

    /// 기다리는 제안 — 오래된 순.
    pub(crate) fn pending_errand_proposals(&self) -> Result<Vec<ErrandProposal>, String> {
        let conn = self.conn();
        expire_errand_proposals(&conn, now_ms()).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(&format!("SELECT {EP_COLS} FROM errand_proposals WHERE status = 'pending' ORDER BY created_at, id"))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_ep)
            .and_then(|it| it.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// 사람이 제안을 승인한다 — 🔴 AI 가 낸 글이 **부탁이 되는 유일한 길**(소켓엔 이 문이 없다). 한 트랜잭션:
    /// 상태 확인 → 부탁 만들기(`approved_at` = 지금) → 제안을 «받음» 으로. `edited` 가 있으면 그 값(사람이 본·고친 글)이 저장된다.
    /// 그냥 승인인데 시각이 이미 지났으면 거절 — 오래 묵은 카드가 승인 즉시 «늦게 실행/건너뜀» 이 되지 않게(고쳐서 받는다).
    pub(crate) fn approve_errand_proposal(&self, id: i64, edited: Option<&ErrandInput>) -> Result<Errand, String> {
        let now = now_ms();
        let errand_id = {
            let mut conn = self.conn();
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            expire_errand_proposals(&tx, now).map_err(|e| e.to_string())?;
            let p = tx
                .query_row(&format!("SELECT {EP_COLS} FROM errand_proposals WHERE id = ?1"), [id], row_to_ep)
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or("없는 제안이에요")?;
            if p.status != "pending" {
                return Err(proposal_gone(&p.status).into());
            }
            let input = match edited {
                Some(e) => e.clone(),
                None => {
                    if p.start_at < now - PAST_MS {
                        return Err("제안한 때가 이미 지났어요 — «고치기» 로 새 때를 정해 주세요".into());
                    }
                    ErrandInput {
                        prompt: p.prompt.clone(),
                        start_at: p.start_at,
                        allowed_tools: p.allowed_tools.clone(),
                        late: p.late.clone(),
                        target: p.target.clone(),
                        ..ErrandInput::default()
                    }
                }
            };
            let (prompt, title) = validate(&input)?;
            let folder_id = input.resume_run_id.map(|r| resume_folder(&tx, r, &input.target)).transpose()?;
            let n = NewErrand {
                title: &title,
                prompt: &prompt,
                start_at: input.start_at,
                target: &input.target,
                allowed_tools: &input.allowed_tools,
                late: &input.late,
                origin: "mcp",
                approved_at: now,
                rrule: Some(input.repeat.as_str()).filter(|r| !r.is_empty()),
                series_id: None,
                carry: input.carry,
                resume_run_id: input.resume_run_id,
                folder_id,
                parent_run_id: p.parent_run_id,
                depth: p.depth,
            };
            let errand_id = insert_errand(&tx, &n, now).map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE errand_proposals SET status = 'approved', decided_at = ?2, errand_id = ?3 WHERE id = ?1",
                params![id, now, errand_id],
            )
            .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            errand_id
        };
        self.get_errand(errand_id)?.ok_or_else(|| "방금 만든 부탁을 못 찾았어요".into())
    }

    pub(crate) fn reject_errand_proposal(&self, id: i64) -> Result<(), String> {
        let now = now_ms();
        let conn = self.conn();
        let n = conn
            .execute(
                "UPDATE errand_proposals SET status = 'rejected', decided_at = ?2 WHERE id = ?1 AND status = 'pending'",
                params![id, now],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            let status: Option<String> = conn
                .query_row("SELECT status FROM errand_proposals WHERE id = ?1", [id], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?;
            return Err(status.map_or("없는 제안이에요", |s| proposal_gone(&s)).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(prompt: &str, at: i64) -> ErrandInput {
        ErrandInput { prompt: prompt.into(), start_at: at, ..ErrandInput::default() }
    }

    #[test]
    fn title_is_first_line_trimmed() {
        assert_eq!(title_of("  어제 커밋 정리해 줘\n자세히"), "어제 커밋 정리해 줘");
        let long = "가".repeat(60);
        assert_eq!(title_of(&long), format!("{}…", "가".repeat(40)));
    }

    #[test]
    fn rejects_bad_input() {
        let s = Store::open_in_memory();
        assert!(s.create_errand(&input("   ", 1)).is_err());
        let mut bad = input("hi", 1);
        bad.allowed_tools = "Bash".into();
        assert!(s.create_errand(&bad).is_err(), "목록에 없는 도구는 못 준다");
        bad = input("hi", 1);
        bad.late = "maybe".into();
        assert!(s.create_errand(&bad).is_err());
        assert!(s.create_errand(&input(&"가".repeat(4001), 1)).is_err());
    }

    #[test]
    fn created_errand_is_approved_waiting_and_listed() {
        let s = Store::open_in_memory();
        let e = s.create_errand(&input("내일 브리핑", 1_000)).unwrap();
        assert!(e.approved && e.run.is_none() && e.target == "claude");
        assert_eq!(s.list_errands(0, 2_000).unwrap().len(), 1);
        assert!(s.list_errands(2_000, 3_000).unwrap().is_empty());
        // 일정 목록·일정 고치기는 부탁을 건드리지 않는다.
        assert!(s.list_events(0, 2_000).unwrap().is_empty());
        assert!(s.delete_event(e.id).is_err());
    }

    #[test]
    fn due_only_when_approved_time_passed_and_never_run() {
        let s = Store::open_in_memory();
        let e = s.create_errand(&input("a", 1_000)).unwrap();
        assert!(s.due_errands(999).unwrap().is_empty(), "아직");
        assert_eq!(s.due_errands(1_000).unwrap().len(), 1);
        // AI 가 낸 부탁(승인 전)은 안 돈다.
        s.conn().execute("UPDATE errands SET approved_at = NULL WHERE item_id = ?1", [e.id]).unwrap();
        assert!(s.due_errands(5_000).unwrap().is_empty());
        s.conn().execute("UPDATE errands SET approved_at = 1 WHERE item_id = ?1", [e.id]).unwrap();
        // 한 번 돌면(건너뜀 포함) 다시 안 걸린다.
        s.begin_run(e.id, "a", 0, Some("건너뜀")).unwrap();
        assert!(s.due_errands(5_000).unwrap().is_empty());
    }

    #[test]
    fn run_lifecycle_and_state() {
        let s = Store::open_in_memory();
        let e = s.create_errand(&input("질문", 1_000)).unwrap();
        let run = s.begin_run(e.id, "질문", 90_000, None).unwrap();
        let st = s.get_errand(e.id).unwrap().unwrap().run.unwrap();
        assert_eq!((st.status.as_str(), st.late_ms, st.sent_text.as_str()), ("running", 90_000, "질문"));
        assert!(s.is_running(e.id).unwrap());
        assert!(s.delete_errand(e.id).is_err(), "도는 중엔 못 지운다");
        let out = Outcome {
            status: "done",
            exit_code: Some(0),
            response: Some("답".into()),
            stderr: None,
            session_id: Some("sid".into()),
        };
        s.finish_run(run, &out).unwrap();
        let st = s.get_errand(e.id).unwrap().unwrap().run.unwrap();
        assert_eq!((st.status.as_str(), st.response.as_deref(), st.read), ("done", Some("답"), false));
        s.mark_run_read(run).unwrap();
        assert!(s.get_errand(e.id).unwrap().unwrap().run.unwrap().read);
        // 돈 부탁은 못 고치지만 지우고 되살릴 수는 있다.
        assert!(s.update_errand(e.id, &input("다른 질문", 1_000)).is_err());
        s.delete_errand(e.id).unwrap();
        assert!(s.get_errand(e.id).unwrap().is_none());
        assert_eq!(s.restore_errand(e.id).unwrap().id, e.id);
    }

    #[test]
    fn claim_uses_current_values_and_refuses_stale_or_double() {
        let s = Store::open_in_memory();
        let none = |_: &Due, _: i64| None;
        let e = s.create_errand(&input("옛 문장", 1_000)).unwrap();
        // 대기하는 동안 고쳤다 → 확보하면 «지금» 문장이 나가고 기록에도 그게 남는다.
        s.update_errand(e.id, &input("고친 문장", 1_000)).unwrap();
        let (run, due, skipped) = s.claim_run(e.id, Claim::Scheduled, none).unwrap().unwrap();
        assert_eq!((due.prompt.as_str(), skipped), ("고친 문장", false));
        assert_eq!(s.get_errand(e.id).unwrap().unwrap().run.unwrap().sent_text, "고친 문장");
        // 이미 돈 부탁은 예약으로 또 못 잡고, 도는 중이거나 «요청 뒤 이미 시작» 이면 수동도 못 잡는다.
        assert!(s.claim_run(e.id, Claim::Scheduled, none).unwrap().is_none());
        assert!(s.claim_run(e.id, Claim::Manual { since: 0 }, none).unwrap().is_none(), "도는 중");
        s.finish_run(run, &Outcome { status: "done", exit_code: Some(0), response: Some("x".into()), stderr: None, session_id: None }).unwrap();
        assert!(s.claim_run(e.id, Claim::Manual { since: 0 }, none).unwrap().is_none(), "요청 뒤에 이미 돌았다");
        assert!(s.claim_run(e.id, Claim::Manual { since: now_ms() + 10_000 }, none).unwrap().is_some(), "다시 실행");
        // 지운 부탁·승인 없는 부탁은 못 잡는다.
        let d = s.create_errand(&input("지울 것", 1_000)).unwrap();
        s.delete_errand(d.id).unwrap();
        assert!(s.claim_run(d.id, Claim::Scheduled, none).unwrap().is_none());
        let u = s.create_errand(&input("승인 전", 1_000)).unwrap();
        s.conn().execute("UPDATE errands SET approved_at = NULL WHERE item_id = ?1", [u.id]).unwrap();
        assert!(s.claim_run(u.id, Claim::Scheduled, none).unwrap().is_none());
        // 요청 뒤에 «건너뜀» 기록만 생겼다면 실제로 돈 게 아니니 수동 요청은 살아 있다.
        let m = s.create_errand(&input("수동", 1_000)).unwrap();
        s.begin_run(m.id, "수동", 0, Some("건너뜀")).unwrap();
        assert!(s.claim_run(m.id, Claim::Manual { since: 0 }, none).unwrap().is_some());
        // 건너뜀 판단은 확보와 같은 트랜잭션 — 기록만 남고 돌지 않는다.
        let k = s.create_errand(&input("놓친 것", 1_000)).unwrap();
        let (_, _, skipped) = s.claim_run(k.id, Claim::Scheduled, |_, _| Some("놓침")).unwrap().unwrap();
        assert!(skipped);
        assert_eq!(s.get_errand(k.id).unwrap().unwrap().run.unwrap().status, "skipped");
    }

    #[test]
    fn long_answer_is_marked_truncated() {
        let s = Store::open_in_memory();
        let e = s.create_errand(&input("q", 1)).unwrap();
        let run = s.begin_run(e.id, "q", 0, None).unwrap();
        let o = Outcome { status: "done", exit_code: Some(0), response: Some("가".repeat(RESPONSE_MAX + 5)), stderr: None, session_id: None };
        s.finish_run(run, &o).unwrap();
        let r = s.get_errand(e.id).unwrap().unwrap().run.unwrap().response.unwrap();
        assert!(r.ends_with("저장했어요)") && r.chars().count() < RESPONSE_MAX + 50);
    }

    #[test]
    fn orphan_runs_fail_on_restart_and_waiting_errand_is_editable() {
        let s = Store::open_in_memory();
        let e = s.create_errand(&input("a", 1_000)).unwrap();
        let moved = s.update_errand(e.id, &input("b", 2_000)).unwrap();
        assert_eq!((moved.prompt.as_str(), moved.start_at), ("b", 2_000));
        s.begin_run(e.id, "b", 0, None).unwrap();
        assert_eq!(s.fail_orphan_runs().unwrap(), 1);
        let r = s.get_errand(e.id).unwrap().unwrap().run.unwrap();
        assert_eq!(r.status, "failed");
        assert!(r.finished_at.is_some());
        assert_eq!(s.runs_since(0).unwrap(), 1);
    }

    fn prop(prompt: &str, at: i64, from: Option<i64>) -> ErrandProposalInput {
        ErrandProposalInput { errand: input(prompt, at), from_run_id: from }
    }
    fn soon() -> i64 {
        now_ms() + 3_600_000
    }

    #[test]
    fn proposal_never_runs_until_a_human_approves() {
        let s = Store::open_in_memory();
        let p = s.add_errand_proposal("claude-code", &prop("어제 커밋 정리", soon(), None)).unwrap();
        assert_eq!((p.status.as_str(), p.depth), ("pending", 0));
        assert!(s.list_errands(0, i64::MAX).unwrap().is_empty(), "제안만으론 부탁이 없다");
        assert!(s.due_errands(i64::MAX).unwrap().is_empty(), "제안은 안 돈다");
        assert_eq!(s.pending_errand_proposals().unwrap().len(), 1);
        let e = s.approve_errand_proposal(p.id, None).unwrap();
        assert!(e.approved && e.run.is_none());
        assert_eq!(s.due_errands(i64::MAX).unwrap().len(), 1, "승인된 뒤에야 돌 차례가 된다");
        let origin: String = s.conn().query_row("SELECT origin FROM items WHERE id = ?1", [e.id], |r| r.get(0)).unwrap();
        assert_eq!(origin, "mcp");
        assert!(s.approve_errand_proposal(p.id, None).is_err(), "두 번 눌러도 부탁은 하나");
        assert!(s.reject_errand_proposal(p.id).is_err());
        assert_eq!(s.list_errands(0, i64::MAX).unwrap().len(), 1);
        assert_eq!(s.get_errand_proposal(p.id).unwrap().unwrap().errand_id, Some(e.id));
    }

    #[test]
    fn edited_approval_stores_what_the_human_saw() {
        let s = Store::open_in_memory();
        let p = s.add_errand_proposal("x", &prop("AI 가 쓴 글", soon(), None)).unwrap();
        let mut mine = input("내가 고친 글", soon() + 1_000);
        mine.allowed_tools = "WebSearch".into();
        let e = s.approve_errand_proposal(p.id, Some(&mine)).unwrap();
        assert_eq!((e.prompt.as_str(), e.allowed_tools.as_str()), ("내가 고친 글", "WebSearch"));
        // 검사에 걸리면 제안은 그대로 기다린다.
        let q = s.add_errand_proposal("x", &prop("b", soon(), None)).unwrap();
        assert!(s.approve_errand_proposal(q.id, Some(&input("  ", 1))).is_err());
        assert_eq!(s.get_errand_proposal(q.id).unwrap().unwrap().status, "pending");
        s.reject_errand_proposal(q.id).unwrap();
        assert!(s.approve_errand_proposal(q.id, None).is_err());
        assert!(s.pending_errand_proposals().unwrap().is_empty());
    }

    #[test]
    fn proposal_checks_input_time_and_caps() {
        let s = Store::open_in_memory();
        assert!(s.add_errand_proposal("x", &prop("  ", soon(), None)).is_err());
        assert!(s.add_errand_proposal("x", &prop("a", now_ms() - 3_600_000, None)).is_err(), "지난 시각");
        assert!(s.add_errand_proposal("x", &prop("a", now_ms() + FAR_MS + 86_400_000, None)).is_err(), "너무 먼 시각");
        let mut bad = prop("a", soon(), None);
        bad.errand.allowed_tools = "Bash".into();
        assert!(s.add_errand_proposal("x", &bad).is_err());
        for i in 0..ERRAND_PENDING_MAX {
            s.add_errand_proposal("x", &prop(&format!("p{i}"), soon(), None)).unwrap();
        }
        assert!(s.add_errand_proposal("x", &prop("넘침", soon(), None)).is_err());
        // 한 주 지난 제안은 만료 → 자리가 나고, 받을 수도 없다.
        s.conn().execute("UPDATE errand_proposals SET created_at = 0 WHERE prompt = 'p0'", []).unwrap();
        assert_eq!(s.pending_errand_proposals().unwrap().len() as i64, ERRAND_PENDING_MAX - 1);
        assert!(s.approve_errand_proposal(1, None).is_err());
        s.add_errand_proposal("x", &prop("자리가 났다", soon(), None)).unwrap();
    }

    #[test]
    fn stale_proposal_needs_a_new_time_to_be_approved() {
        let s = Store::open_in_memory();
        let p = s.add_errand_proposal("x", &prop("a", soon(), None)).unwrap();
        // 카드가 묵는 사이 제안한 때가 지났다.
        s.conn().execute("UPDATE errand_proposals SET start_at = ?1 WHERE id = ?2", params![now_ms() - 3_600_000, p.id]).unwrap();
        assert!(s.approve_errand_proposal(p.id, None).is_err());
        assert_eq!(s.get_errand_proposal(p.id).unwrap().unwrap().status, "pending");
        s.approve_errand_proposal(p.id, Some(&input("a", soon()))).unwrap();
    }

    #[test]
    fn chain_depth_is_limited_to_three() {
        let s = Store::open_in_memory();
        // 사람이 만든 부탁(깊이 0)이 돌았다.
        let mut e = s.create_errand(&input("뿌리", 1_000)).unwrap();
        let mut run = s.begin_run(e.id, "뿌리", 0, None).unwrap();
        for want in 1..=DEPTH_MAX {
            let p = s.add_errand_proposal("x", &prop(&format!("{want}단계"), soon(), Some(run))).unwrap();
            assert_eq!(p.depth, want);
            e = s.approve_errand_proposal(p.id, None).unwrap();
            run = s.begin_run(e.id, &e.prompt, 0, None).unwrap();
        }
        let err = s.add_errand_proposal("x", &prop("4단계", soon(), Some(run))).unwrap_err();
        assert!(err.contains("3번"), "{err}");
        assert!(s.add_errand_proposal("x", &prop("없는 실행", soon(), Some(9_999))).is_err());
        // 답을 안 밝힌 제안은 0 단계 — 그래도 사람 승인은 필요하다.
        assert_eq!(s.add_errand_proposal("x", &prop("새 뿌리", soon(), None)).unwrap().depth, 0);
    }

    #[test]
    fn schema_5_db_upgrades_keeping_errands() {
        // 개발 5 사용자의 DB(스키마 4)가 개발 6 앱에서 열린다.
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = rusqlite::Connection::open(dir.path().join("ouro.db")).unwrap();
            let n = crate::store::MIGRATIONS.len() - 1;
            for (i, sql) in crate::store::MIGRATIONS.iter().take(n).enumerate() {
                conn.execute_batch(&format!("BEGIN; {sql}; PRAGMA user_version = {}; COMMIT;", i + 1)).unwrap();
            }
        }
        let s = Store::open(dir.path()).unwrap();
        s.add_errand_proposal("x", &prop("a", soon(), None)).unwrap();
    }

    // ── 개발 7: 반복·이어서·대상 ──

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        use chrono::TimeZone;
        chrono::Local.with_ymd_and_hms(y, m, d, h, min, 0).earliest().unwrap().timestamp_millis()
    }

    fn done(s: &Store, run: i64, session: &str) {
        let o = Outcome { status: "done", exit_code: Some(0), response: Some("답".into()), stderr: None, session_id: Some(session.into()) };
        s.finish_run(run, &o).unwrap();
    }

    #[test]
    fn next_occurrence_keeps_wall_clock_and_rule() {
        let mon9 = local(2026, 10, 5, 9, 0); // 월요일
        assert_eq!(next_occurrence("FREQ=DAILY", mon9, mon9), Some(local(2026, 10, 6, 9, 0)));
        assert_eq!(next_occurrence("FREQ=WEEKLY", mon9, mon9), Some(local(2026, 10, 12, 9, 0)));
        let fri9 = local(2026, 10, 9, 9, 0);
        assert_eq!(next_occurrence("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", fri9, fri9), Some(local(2026, 10, 12, 9, 0)), "금 → 월");
        // 사흘 놓쳤으면 «지금 이후» 첫 회차 하나 — 몰아 돌리지 않는다.
        assert_eq!(next_occurrence("FREQ=DAILY", mon9, local(2026, 10, 8, 10, 0)), Some(local(2026, 10, 9, 9, 0)));
        assert_eq!(next_occurrence("FREQ=DAILY", mon9, local(2026, 10, 8, 8, 0)), Some(local(2026, 10, 8, 9, 0)));
        assert_eq!(next_occurrence("FREQ=MONTHLY", mon9, mon9), None, "모르는 규칙은 없다");
    }

    #[test]
    fn repeat_validation() {
        let s = Store::open_in_memory();
        let mut i = input("브리핑", now_ms() + 60_000);
        i.repeat = "FREQ=YEARLY".into();
        assert!(s.create_errand(&i).is_err());
        i.repeat = String::new();
        i.carry = true;
        assert!(s.create_errand(&i).is_err(), "반복 없이 «지난 대화» 는 없다");
        i.carry = false;
        i.target = "gemini".into();
        assert!(s.create_errand(&i).is_err());
    }

    #[test]
    fn recurring_errand_spawns_next_on_first_claim_only() {
        let s = Store::open_in_memory();
        let at = now_ms() - 3 * 24 * 60 * 60 * 1000; // 사흘 전 회차가 아직 안 돌았다(맥이 잤다)
        let mut i = input("아침 브리핑", at);
        i.repeat = "FREQ=DAILY".into();
        i.carry = true;
        let head = s.create_errand(&i).unwrap();
        assert_eq!((head.repeat.as_str(), head.series), ("FREQ=DAILY", true));
        let (_, _, skipped) = s.claim_run(head.id, Claim::Scheduled, |_, _| Some("놓침")).unwrap().unwrap();
        assert!(skipped);
        let all = s.list_errands(0, i64::MAX).unwrap();
        assert_eq!(all.len(), 2, "놓친 회차 하나 + 다음 회차 하나");
        let next = all.iter().find(|e| e.id != head.id).unwrap();
        assert!(next.start_at > now_ms() && next.repeat == "FREQ=DAILY" && next.approved && next.carry);
        assert_eq!(s.get_errand(head.id).unwrap().unwrap().repeat, "", "규칙은 다음 회차로 넘어갔다");
        // 같은 회차를 다시 돌려도(수동) 또 만들지 않는다.
        s.claim_run(head.id, Claim::Manual { since: 0 }, |_, _| None).unwrap().unwrap();
        assert_eq!(s.list_errands(0, i64::MAX).unwrap().len(), 2);
        // 다음 회차를 지우면 반복이 멈춘다 — 돌 게 없다.
        s.delete_errand(next.id).unwrap();
        assert!(s.next_errand_at(0).unwrap().is_none());
    }

    #[test]
    fn carry_resumes_latest_answer_of_the_series_in_one_folder() {
        let s = Store::open_in_memory();
        let mut i = input("오늘 할 일 정리", now_ms() - 1_000);
        i.repeat = "FREQ=DAILY".into();
        i.carry = true;
        let head = s.create_errand(&i).unwrap();
        let (run, d, _) = s.claim_run(head.id, Claim::Scheduled, |_, _| None).unwrap().unwrap();
        assert_eq!((d.session, d.folder), (None, head.id), "첫 회차는 새 대화");
        done(&s, run, "sess-1");
        let next = s.list_errands(0, i64::MAX).unwrap().into_iter().find(|e| e.id != head.id).unwrap();
        // 다음 회차를 지금으로 당겨 돌린다.
        s.conn().execute("UPDATE items SET start_at = ?1 WHERE id = ?2", params![now_ms() - 1, next.id]).unwrap();
        let (run2, d2, _) = s.claim_run(next.id, Claim::Scheduled, |_, _| None).unwrap().unwrap();
        assert_eq!((d2.session.as_deref(), d2.folder), (Some("sess-1"), head.id), "같은 폴더에서 지난 대화를 잇는다");
        assert!(s.get_errand(next.id).unwrap().unwrap().run.unwrap().resumed);
        let _ = run2;
        // carry 가 꺼진 반복은 새 대화.
        let mut j = input("새로", now_ms() - 1_000);
        j.repeat = "FREQ=DAILY".into();
        let h2 = s.create_errand(&j).unwrap();
        assert_eq!(s.claim_run(h2.id, Claim::Scheduled, |_, _| None).unwrap().unwrap().1.session, None);
    }

    #[test]
    fn resume_needs_an_answer_of_the_same_target() {
        let s = Store::open_in_memory();
        let p = s.create_errand(&input("어제 커밋 정리", now_ms() - 1_000)).unwrap();
        let mut follow = input("그중 제일 큰 것만 다시", now_ms() + 60_000);
        let (run, _, _) = s.claim_run(p.id, Claim::Scheduled, |_, _| None).unwrap().unwrap();
        follow.resume_run_id = Some(run);
        assert!(s.create_errand(&follow).is_err(), "답이 오기 전엔 못 잇는다");
        done(&s, run, "abc-123");
        assert!(s.get_errand(p.id).unwrap().unwrap().run.unwrap().can_resume);
        follow.target = "codex".into();
        assert!(s.create_errand(&follow).is_err(), "Claude 대화를 Codex 로 이을 수 없다");
        follow.target = "claude".into();
        follow.repeat = "FREQ=DAILY".into();
        assert!(s.create_errand(&follow).is_err(), "이어서 + 반복은 안 된다");
        follow.repeat = String::new();
        let f = s.create_errand(&follow).unwrap();
        assert_eq!(f.resume_title.as_deref(), Some("어제 커밋 정리"));
        // 고치기로 받는 쪽을 바꿀 수 없다(이어서는 만들 때 정해진 대로).
        let mut edit = follow.clone();
        edit.target = "codex".into();
        edit.resume_run_id = None;
        assert!(s.update_errand(f.id, &edit).is_err());
        s.conn().execute("UPDATE items SET start_at = ?1 WHERE id = ?2", params![now_ms() - 1, f.id]).unwrap();
        let (_, d, _) = s.claim_run(f.id, Claim::Scheduled, |_, _| None).unwrap().unwrap();
        assert_eq!((d.session.as_deref(), d.folder, d.prompt.as_str()), (Some("abc-123"), p.id, "그중 제일 큰 것만 다시"));
    }

    #[test]
    fn ai_proposals_are_one_shot_but_may_target_codex() {
        let s = Store::open_in_memory();
        let mut e = input("요약", now_ms() + 60_000);
        e.repeat = "FREQ=DAILY".into();
        assert!(s.add_errand_proposal("x", &ErrandProposalInput { errand: e.clone(), from_run_id: None }).is_err());
        e.repeat = String::new();
        e.target = "codex".into();
        let p = s.add_errand_proposal("x", &ErrandProposalInput { errand: e, from_run_id: None }).unwrap();
        let made = s.approve_errand_proposal(p.id, None).unwrap();
        assert_eq!(made.target, "codex");
    }
}
