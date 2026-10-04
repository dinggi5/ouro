// 디스패처 — 때가 된 부탁을 `claude -p`·`codex exec` 로 보내고, 답을 받아 `runs` 에 적는다(PLAN §9).
//
// 설계 결정:
//   · **한 번에 하나.** 일꾼 스레드 하나가 줄 세워 돌린다 — 구독 한도·맥 부하를 부탁이 몰려도 지키고, 로직이 단순하다.
//     도는 동안 새로 온 `poke`·`run_now` 는 채널에 쌓였다가 끝난 뒤 처리된다.
//   · **기본은 대화만.** `--tools ""`(도구 없음) + `--strict-mcp-config`(MCP 서버 없음) + `--setting-sources ""`(사용자 설정의
//     허용 목록·훅 안 읽음). 도구는 부탁 카드에서 고른 것만 `--tools`·`--allowedTools` 로 준다(PLAN §9-2). 부탁마다
//     `~/.ouro/runs/<id>/` 에서 돌아 다른 폴더를 건드릴 일이 없다.
//   · **원문은 표준입력으로.** 인자로 주면 `-` 로 시작하는 부탁이 플래그로 읽힌다.
//   · **보낸 원문이 곧 부탁 문장이다.** 일정·AI 답이 섞여 들어가지 않는다 — 바깥에서 온 글은 문장이 될 수 없다(CLAUDE.md).
//   · 놓친 부탁(맥이 자고 있었다): «늦게 실행» 은 12시간까지, «건너뜀» 은 5분을 넘기면. 건너뛰어도 기록 한 줄을 남겨 다시 안 걸린다.
//   · 시간 제한 10분, 하루 실행 상한 50(예약 실행만 — 사람이 누른 «지금 실행» 은 세지만 막지 않는다, `errands::DAILY_CAP`).
//     프로세스 그룹째 멈춘다.
//   · **앱이 강제 종료돼도 자식이 안 남는다**(개발 9): CLI 를 작은 `sh` 껍데기로 띄우고, 껍데기가 같은 그룹에 감시꾼을 하나 둔 뒤
//     `exec` 로 CLI 가 된다. 감시꾼은 앱 pid 가 사라지면 그룹째 죽인다 — 강제 종료·패닉·크래시엔 `shutdown` 이 못 돌기 때문이다.
//     그래서 켤 때 «도는 중» 을 «끊김» 으로 닫아도 옛 프로세스가 뒤에서 계속 돌며 다시 실행과 겹치지 않는다(코덱스 개발 6·7·8 P1).
//   · 앱이 꺼진 채 남은 «도는 중» 은 켤 때 «끊김» 으로 닫는다 — 프로세스는 위 감시꾼이 이미 치웠다.
//   · `claude` 는 Finder 로 켠 앱의 좁은 PATH 에선 안 보인다 — 흔한 설치 자리, 안 되면 로그인 셸의 PATH 에서 찾는다(개발 5 첫 확인 거리).
//     자식에게 주는 PATH 도 로그인 셸 것을 잇는다 — `#!/usr/bin/env node` 인 CLI 가 nvm 의 node 를 찾게(개발 9, 코덱스 개발 8 P2).
//   · **Codex(개발 7)도 «대화만»** 이 기본이다: `--ignore-user-config`(사용자 config 의 MCP 서버·모델·권한을 안 읽음) +
//     `--ignore-rules` + 읽기 전용 샌드박스 + 셸·앱·플러그인·브라우저·컴퓨터 조작 등 도구 기능을 `-c features.X=false` 로 끈다.
//     `--disable X` 가 아니라 `-c` 인 이유: 모르는 기능 이름을 `--disable` 은 **오류로 끝내고** `-c` 는 경고만 한다(2026-10-03 실측,
//     codex 0.159) — Codex 가 기능 하나를 없애는 업데이트로 모든 Codex 부탁이 멈추지 않게. 웹 검색은 `web_search` 를 live/disabled 로.
//   · **이어서**(개발 7): `claude --resume <id>` · `codex exec resume <id>`. 대화 번호는 AI 쪽에서 온 글이라 모양을 확인하고 쓴다
//     (`-` 로 시작하면 플래그로 읽힌다). Claude Code 는 대화를 작업 폴더별로 두므로 이을 땐 같은 폴더(`Due::folder`)에서 돈다.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::alerts::show_notification;
use crate::errands::{title_of, Claim, Due, Outcome};
use crate::store::{self, now_ms, Store};

const MAX_NAP: Duration = Duration::from_secs(30);
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SKIP_GRACE_MS: i64 = 5 * 60 * 1000;
const LATE_RUN_MAX_MS: i64 = 12 * 60 * 60 * 1000;
/// 읽어 둘 출력 상한(바이트). 넘는 건 버리되 계속 비워 준다(안 비우면 자식이 파이프에 막힌다).
/// 답은 어차피 10만 자에서 자른다(`errands::RESPONSE_MAX`) — 이 상한은 JSON 이 잘려 정상 답을 실패로 읽는 일이 없게 넉넉히.
const OUTPUT_CAP: usize = 8 * 1024 * 1024;
/// 감시꾼이 앱이 살아 있나 보는 간격(초). 앱이 죽고 길어야 이만큼 뒤에 자식이 멈춘다.
const WATCH_SECS: u32 = 2;

pub(crate) type OnChange = Arc<dyn Fn() + Send + Sync>;

enum Msg {
    Poke,
    /// (부탁, 누른 시각)
    RunNow(i64, i64),
}

type Stops = Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>;

/// 디스패처 손잡이.
pub(crate) struct Dispatcher {
    tx: Sender<Msg>,
    stops: Stops,
    /// «지금 실행» 으로 줄에 선 부탁 — 두 번 눌러도 한 번만.
    queued: Arc<Mutex<HashSet<i64>>>,
    /// 끄는 중 — 새 실행을 시작하지 않는다.
    closing: Arc<AtomicBool>,
    /// 업데이트 설치 중 — 새 실행을 시작하지 않는다(update.rs). 플래그를 **잠금 안에** 둔 이유: 일꾼은 «잡혀 있나 확인 → 실행 확보»
    /// 를 이 잠금을 쥔 채 하고, 설치는 이 잠금을 쥐고 플래그를 세운 **뒤에** «도는 부탁이 있나» 를 DB 에 묻는다 —
    /// 그래서 확인과 확보 사이에 끼어든 실행이 재시작에 휩쓸리는 틈이 없다(코덱스 개발 8 1차 P1).
    hold: Arc<Mutex<bool>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Dispatcher {
    /// 부탁이 새로 생기거나 고쳐졌다 — 잠든 일꾼을 깨워 다음 시각을 다시 본다.
    pub(crate) fn poke(&self) {
        let _ = self.tx.send(Msg::Poke);
    }

    /// 예약과 무관하게 지금 한 번 돌린다(다시 실행 포함). 줄에 섰으면 true.
    pub(crate) fn run_now(&self, id: i64) -> bool {
        if !lock(&self.queued).insert(id) {
            return false;
        }
        let _ = self.tx.send(Msg::RunNow(id, now_ms()));
        true
    }

    /// 앱을 끌 때 — 도는 실행을 전부 멈추고(프로세스 그룹째) 정리될 때까지 잠깐 기다린다. 안 하면 `claude` 가 앱보다 오래 산다.
    pub(crate) fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        for flag in lock(&self.stops).values() {
            flag.store(true, Ordering::SeqCst);
        }
        for _ in 0..30 {
            if lock(&self.stops).is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// 새 실행을 막는다. 이 함수가 돌아온 뒤엔 일꾼이 새로 확보하는 실행이 없다 — 막 확보하던 것은 이미 DB 에 «도는 중» 으로 있다.
    pub(crate) fn hold(&self) {
        *lock(&self.hold) = true;
    }

    /// `hold` 를 푼다(설치가 실패했을 때). 막혀 있던 부탁은 다음 바퀴에 돈다.
    pub(crate) fn release(&self) {
        *lock(&self.hold) = false;
        self.poke();
    }

    /// 도는 실행을 멈춘다. 돌고 있었으면 true.
    pub(crate) fn stop(&self, run_id: i64) -> bool {
        match lock(&self.stops).get(&run_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Decision {
    Run,
    Skip(&'static str),
}

/// 놓친 정도에 따라 돌릴지 건너뛸지 (순수 — 테스트 가능). 수동 실행은 여기를 거치지 않는다.
fn decide(late: &str, late_ms: i64) -> Decision {
    if late_ms > LATE_RUN_MAX_MS {
        Decision::Skip("12시간이 넘게 늦어서 건너뛰었어요")
    } else if late == "skip" && late_ms > SKIP_GRACE_MS {
        Decision::Skip("예약한 시각에 맥이 쉬고 있어서 건너뛰었어요")
    } else {
        Decision::Run
    }
}

/// `claude` 에 줄 인자 (순수). 부탁 문장은 표준입력으로 간다. `session` = 이을 대화.
fn claude_args(allowed_tools: &str, session: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = ["-p", "--output-format", "json", "--strict-mcp-config", "--permission-mode", "default"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // 도구를 하나도 안 주면 한 번에 끝나야 하고, 웹 검색은 검색→읽기→답으로 몇 걸음이 든다.
    a.extend(["--setting-sources".into(), String::new(), "--tools".into(), allowed_tools.into()]);
    a.extend(["--max-turns".into(), if allowed_tools.is_empty() { "3" } else { "12" }.into()]);
    if !allowed_tools.is_empty() {
        a.extend(["--allowedTools".into(), allowed_tools.into()]);
    }
    if let Some(sid) = session {
        a.extend(["--resume".into(), sid.into()]);
    }
    a
}

/// Codex 에서 끄는 기능 — 부탁은 «대화만»(+ 고르면 웹 검색)이다. 셸·파일 보기·앱·플러그인·브라우저·컴퓨터 조작·기억·하위 에이전트 등.
const CODEX_OFF: [&str; 21] = [
    "shell_tool",
    "unified_exec",
    "shell_snapshot",
    "apps",
    "plugins",
    "remote_plugin",
    "browser_use",
    "browser_use_external",
    "in_app_browser",
    "computer_use",
    "memories",
    "multi_agent",
    "image_generation",
    "hooks",
    "view_image",
    "skill_mcp_dependency_install",
    "tool_suggest",
    "goals",
    "daemon_auto_start",
    "workspace_dependencies",
    "sleep_tool",
];

/// 대화 번호가 인자로 써도 되는 모양인가 — 영문·숫자·`-`·`_` 만, 플래그처럼 `-` 로 시작하지 않게.
fn safe_session(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `codex` 에 줄 인자 (순수). 부탁 문장은 표준입력(`-`)으로 간다.
fn codex_args(allowed_tools: &str, session: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = vec!["exec".into()];
    if session.is_some() {
        a.push("resume".into());
    }
    a.extend(["--ignore-user-config", "--ignore-rules", "--skip-git-repo-check", "--json"].map(String::from));
    let web = if allowed_tools == "WebSearch" { "live" } else { "disabled" };
    a.extend(["-c".into(), "sandbox_mode=\"read-only\"".into(), "-c".into(), format!("web_search=\"{web}\"")]);
    for f in CODEX_OFF {
        a.extend(["-c".into(), format!("features.{f}=false")]);
    }
    if let Some(sid) = session {
        a.push(sid.into());
    }
    a.push("-".into());
    a
}

/// 부탁을 받는 쪽의 실행 파일을 찾는다. 디버그 빌드에선 `OURO_CLAUDE`·`OURO_CODEX` 로 바꿀 수 있다(테스트용 가짜 실행 파일).
fn find_cli(target: &str) -> Result<PathBuf, String> {
    let (name, label) = if target == "codex" { ("codex", "Codex(`codex`)") } else { ("claude", "Claude Code(`claude`)") };
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os(if target == "codex" { "OURO_CODEX" } else { "OURO_CLAUDE" }) {
        return Ok(PathBuf::from(p));
    }
    let home = dirs::home_dir();
    let mut cands: Vec<PathBuf> = vec![];
    if let Some(h) = &home {
        let rels: &[&str] = if target == "codex" {
            &[".local/bin/codex", ".npm-global/bin/codex", ".bun/bin/codex", ".volta/bin/codex"]
        } else {
            &[".local/bin/claude", ".claude/local/claude", ".npm-global/bin/claude", ".bun/bin/claude"]
        };
        cands.extend(rels.iter().map(|r| h.join(r)));
    }
    cands.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(|d| Path::new(d).join(name)));
    // 로그인 셸의 PATH — nvm 같은 곳에 깔았을 때.
    if let Some(lp) = login_path() {
        cands.extend(std::env::split_paths(&lp).map(|d| d.join(name)));
    }
    cands
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("{label}를 못 찾았어요 — 설치돼 있고 로그인돼 있어야 해요"))
}

/// 로그인 셸의 PATH. Finder 로 켠 앱의 PATH 는 `/usr/bin:/bin:…` 뿐이라 nvm·volta 같은 곳(셸 rc 가 PATH 에 넣는다)이 안 보인다.
/// 처음 필요할 때 셸을 한 번 띄워(최대 5초) 앱이 켜져 있는 동안 기억한다. 못 얻으면 기억하지 않고 다음에 다시 묻는다.
fn login_path() -> Option<String> {
    static CACHE: Mutex<Option<String>> = Mutex::new(None);
    if let Some(p) = lock(&CACHE).clone() {
        return Some(p);
    }
    let p = ask_login_shell_path()?;
    *lock(&CACHE) = Some(p.clone());
    Some(p)
}

fn ask_login_shell_path() -> Option<String> {
    const MARK: &str = "__OURO_PATH__=";
    // 대화형(-i)은 rc 파일 잡음이 섞이니 표식이 붙은 줄만 쓴다.
    let mut child = Command::new("/bin/zsh")
        .args(["-lic", &format!("printf '\\n{MARK}%s\\n' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| eprintln!("ouro: 로그인 셸을 못 띄웠어요 — {e}"))
        .ok()?;
    let out = drain(child.stdout.take().unwrap());
    // rc 파일이 입력을 기다리며 멈출 수 있다 — 5초만 기다리고 접는다(코덱스 개발 5: 이 대기는 중단 버튼으로도 안 끊긴다).
    let started = Instant::now();
    while child.try_wait().ok().flatten().is_none() {
        if started.elapsed() > Duration::from_secs(5) {
            kill_group(child.id());
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    kill_group(child.id()); // 셸이 끝난 뒤 남은 손주가 파이프를 붙들지 않게
    let stdout = String::from_utf8_lossy(&out.finish(Duration::from_secs(2))).into_owned();
    stdout.lines().rev().find_map(|l| l.strip_prefix(MARK)).map(str::to_string).filter(|p| !p.is_empty())
}

/// 자식에게 줄 PATH (순수 — 테스트 가능): CLI 가 있는 폴더(링크면 실제 폴더도) → 로그인 셸 PATH → 앱의 PATH → 기본 자리. 겹치면 앞의 것만.
fn child_path(exe: &Path, login: Option<&str>, inherited: Option<&str>) -> String {
    let mut dirs: Vec<PathBuf> = vec![];
    dirs.extend(exe.parent().map(Path::to_path_buf));
    dirs.extend(std::fs::canonicalize(exe).ok().and_then(|p| p.parent().map(Path::to_path_buf)));
    for list in [login, inherited, Some("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")].into_iter().flatten() {
        dirs.extend(std::env::split_paths(list));
    }
    let mut seen = HashSet::new();
    dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `codex exec --json` 의 출력(JSONL 이벤트) → 결과. 답 = 마지막 `agent_message`, 대화 번호 = `thread.started` 의 `thread_id`.
/// `item` 의 `error` 는 경고(모르는 설정 등)라 실패로 치지 않는다 — 실패는 `turn.failed` 와 맨 위 `error` 이벤트다.
fn parse_codex_output(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Outcome {
    let (mut text, mut session, mut why): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    for v in stdout.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok()) {
        match v["type"].as_str() {
            Some("thread.started") => session = v["thread_id"].as_str().map(str::to_string),
            Some("item.completed") if v["item"]["type"] == "agent_message" => text = v["item"]["text"].as_str().map(str::to_string),
            Some("turn.failed") => why = v["error"]["message"].as_str().map(str::to_string).or(why),
            Some("error") => why = why.or(v["message"].as_str().map(str::to_string)),
            _ => {}
        }
    }
    // 오류 문장 안에 API 오류 JSON 이 통째로 들어 있기도 하다 — 사람이 읽을 `error.message` 만 꺼낸다.
    let why = why.map(|w| {
        serde_json::from_str::<serde_json::Value>(&w)
            .ok()
            .and_then(|j| j["error"]["message"].as_str().map(str::to_string))
            .unwrap_or(w)
    });
    let ok = exit_code == Some(0) && why.is_none() && text.as_ref().is_some_and(|t| !t.trim().is_empty());
    if ok {
        return Outcome { status: "done", exit_code, response: text, stderr: Some(stderr.to_string()), session_id: session };
    }
    let reason = why
        .or_else(|| Some(stderr.trim().to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| if text.is_none() { "답이 비어 있어요".into() } else { "Codex 가 오류로 끝났어요".into() });
    Outcome { status: "failed", exit_code, response: None, stderr: Some(reason), session_id: session }
}

/// `claude --output-format json` 의 출력 → 결과. 종료 상태와 `is_error` 를 같이 본다(로그인 안 됨 같은 오류도 종료코드 1 + JSON 이다).
fn parse_output(stdout: &str, stderr: &str, exit_code: Option<i32>) -> Outcome {
    let parsed: Option<serde_json::Value> = serde_json::from_str(stdout.trim()).ok();
    // 어떤 버전은 이벤트 배열을 준다 — 마지막 result 를 쓴다.
    let obj = match parsed {
        Some(serde_json::Value::Array(a)) => a.into_iter().rev().find(|v| v["type"] == "result"),
        other => other,
    };
    let text = obj.as_ref().and_then(|v| v["result"].as_str()).map(str::to_string);
    let is_error = obj.as_ref().is_some_and(|v| v["is_error"].as_bool().unwrap_or(false));
    let session_id = obj.as_ref().and_then(|v| v["session_id"].as_str()).map(str::to_string);
    let ok = exit_code == Some(0) && !is_error && text.as_ref().is_some_and(|t| !t.trim().is_empty());
    if ok {
        return Outcome { status: "done", exit_code, response: text, stderr: Some(stderr.to_string()), session_id };
    }
    // 실패 — 사람이 볼 이유를 stderr 칸에 모은다(오류 문장은 JSON 의 result 에 오기도 한다).
    let mut why = stderr.trim().to_string();
    if why.is_empty() {
        why = match (&text, stdout.trim()) {
            (Some(t), _) if !t.trim().is_empty() => t.clone(),
            (_, raw) if !raw.is_empty() => raw.chars().take(500).collect(),
            _ => "답이 비어 있어요".to_string(),
        };
    }
    Outcome { status: "failed", exit_code, response: None, stderr: Some(why), session_id }
}

/// 파이프 읽기. 끝나길 `finish` 가 기다리되 시간이 지나면 **그때까지 받은 것**을 돌려준다(코덱스 개발 5 — 분리된 손주가 파이프를
/// 붙들어도 정상 답을 버리지 않고, 일꾼도 안 막힌다).
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    handle: std::thread::JoinHandle<()>,
}

fn drain(mut r: impl Read + Send + 'static) -> Drain {
    let buf: Arc<Mutex<Vec<u8>>> = Arc::default();
    let b = buf.clone();
    let handle = std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut g = lock(&b);
            let room = OUTPUT_CAP.saturating_sub(g.len());
            g.extend_from_slice(&chunk[..n.min(room)]);
        }
    });
    Drain { buf, handle }
}

impl Drain {
    fn finish(self, wait: Duration) -> Vec<u8> {
        let t = Instant::now();
        while !self.handle.is_finished() && t.elapsed() < wait {
            std::thread::sleep(Duration::from_millis(20));
        }
        let v = lock(&self.buf).clone();
        v
    }
}

fn kill_group(pid: u32) {
    // 프로세스 그룹 전체 — `claude` 가 띄운 자식(검색 도구 등)까지.
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

/// 감시 껍데기(`sh -c`). `$1` = 앱 pid, 나머지 = CLI 와 인자. 그룹 리더인 껍데기(`$$`)가 감시꾼을 같은 그룹에 띄우고 `exec` 로 CLI 가 된다 —
/// 그래서 CLI 의 pid = 그룹 번호이고, 평소의 `kill_group` 이 감시꾼까지 같이 치운다. 감시꾼은 앱이 사라지면 그룹째 죽인다.
/// 감시꾼의 표준 입출력은 `/dev/null` — 출력 파이프를 붙들어 답 읽기를 막지 않게.
const WATCH_SH: &str = r#"p=$1; shift; g=$$
( while kill -0 "$p" 2>/dev/null; do sleep __SECS__; done; kill -KILL -- "-$g" ) </dev/null >/dev/null 2>&1 &
exec "$@""#;

/// 프로세스를 돌려 끝날 때까지 기다린다. `stop` 이 켜지면 멈추고(stopped), `timeout` 을 넘으면 멈춘다(failed).
/// `parent` 가 사라지면 감시꾼이 프로세스 그룹을 죽인다(`WATCH_SH`) — 보통은 이 앱의 pid.
#[allow(clippy::too_many_arguments)]
fn run_process(
    exe: &Path,
    args: &[String],
    prompt: &str,
    cwd: &Path,
    path_env: &str,
    parent: u32,
    stop: &AtomicBool,
    timeout: Duration,
    parse: fn(&str, &str, Option<i32>) -> Outcome,
) -> Outcome {
    let fail = |msg: String| Outcome { status: "failed", exit_code: None, response: None, stderr: Some(msg), session_id: None };
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // 확보한 뒤 찾는 사이(로그인 셸 최대 5초)에 중단을 눌렀으면 띄우지 않는다(코덱스 개발 6 P1).
    if stop.load(Ordering::SeqCst) {
        return Outcome { status: "stopped", exit_code: None, response: None, stderr: None, session_id: None };
    }
    // 껍데기는 늘 뜨므로 «없는 실행 파일» 은 여기서 가린다 — 안 그러면 `exec` 의 127 이 알 수 없는 실패로 읽힌다.
    if !exe.is_file() {
        return fail(format!("`{name}` 를 못 띄웠어요: 파일이 없어요"));
    }
    let mut child = match Command::new("/bin/sh")
        .arg("-c")
        .arg(WATCH_SH.replace("__SECS__", &WATCH_SECS.to_string()))
        .arg("ouro-run")
        .arg(parent.to_string())
        .arg(exe)
        .args(args)
        .current_dir(cwd)
        .env("PATH", path_env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return fail(format!("`{name}` 를 못 띄웠어요: {e}")),
    };
    let pid = child.id();
    if let Some(mut stdin) = child.stdin.take() {
        let text = prompt.to_string();
        // 큰 문장이 파이프에 걸려도 기다림 루프를 막지 않게 따로. 닫으면 `claude` 가 입력 끝을 안다.
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let (out, err) = (drain(child.stdout.take().unwrap()), drain(child.stderr.take().unwrap()));
    let started = Instant::now();
    let mut stopped = false;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(e) => return fail(format!("실행 상태를 못 읽었어요: {e}")),
        }
        if stop.load(Ordering::SeqCst) {
            stopped = true;
        } else if started.elapsed() > timeout {
            timed_out = true;
        }
        if stopped || timed_out {
            kill_group(pid);
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    // 자식이 출력 파이프를 붙든 손주를 남기면 읽기가 안 끝난다 — 프로세스 그룹을 한 번 더 치고 3초만 기다린다(코덱스 개발 5).
    kill_group(pid);
    let stdout_bytes = out.finish(Duration::from_secs(3));
    let stderr = String::from_utf8_lossy(&err.finish(Duration::from_secs(3))).into_owned();
    if stopped {
        return Outcome { status: "stopped", exit_code: None, response: None, stderr: None, session_id: None };
    }
    if timed_out {
        return fail(format!("{}분을 넘겨 멈췄어요", timeout.as_secs() / 60));
    }
    if stdout_bytes.len() >= OUTPUT_CAP {
        return fail(format!("답이 너무 길어서({}MB 넘음) 읽지 못했어요", OUTPUT_CAP / 1024 / 1024));
    }
    parse(&String::from_utf8_lossy(&stdout_bytes), &stderr, status.and_then(|s| s.code()))
}

fn notify(title: &str, body: &str) {
    show_notification(title, body, || {});
}

/// 확보한 실행(`claim_run`)을 돌린다. `d` 는 **확보 시점의 현재 값**이다.
fn execute(store: &Store, dir: &Path, stops: &Stops, closing: &AtomicBool, on_change: &OnChange, run_id: i64, d: &Due) {
    let title = title_of(&d.prompt);
    // 끄는 중에 막 시작한 실행은 시작하자마자 멈춘다 — `shutdown` 이 못 본 플래그가 없게(코덱스 개발 5 2차).
    let flag = Arc::new(AtomicBool::new(false));
    lock(stops).insert(run_id, flag.clone());
    if closing.load(Ordering::SeqCst) {
        flag.store(true, Ordering::SeqCst);
    }
    on_change();

    // 이을 땐 그 대화가 사는 폴더에서 — Claude Code 는 대화를 작업 폴더별로 둔다.
    let workdir = dir.join("runs").join(d.folder.to_string());
    let codex = d.target == "codex";
    let session = d.session.as_deref();
    let me = std::process::id();
    let path_of = |exe: &Path| child_path(exe, login_path().as_deref(), std::env::var("PATH").ok().as_deref());
    let outcome = match (find_cli(&d.target), store::create_private_dir(&workdir)) {
        (Err(e), _) | (_, Err(e)) => Outcome { status: "failed", exit_code: None, response: None, stderr: Some(e), session_id: None },
        _ if session.is_some_and(|s| !safe_session(s)) => Outcome {
            status: "failed",
            exit_code: None,
            response: None,
            stderr: Some("이을 대화 번호 모양이 이상해서 보내지 않았어요".into()),
            session_id: None,
        },
        (Ok(exe), Ok(())) if codex => {
            let args = codex_args(&d.allowed_tools, session);
            run_process(&exe, &args, &d.prompt, &workdir, &path_of(&exe), me, &flag, RUN_TIMEOUT, parse_codex_output)
        }
        (Ok(exe), Ok(())) => {
            let args = claude_args(&d.allowed_tools, session);
            run_process(&exe, &args, &d.prompt, &workdir, &path_of(&exe), me, &flag, RUN_TIMEOUT, parse_output)
        }
    };
    // 답을 못 적으면 몇 번 더 — 그래도 안 되면 «왔어요» 라고 하지 않는다(DB 에는 «도는 중» 이 남고 다음 켤 때 «끊김» 으로 닫힌다).
    let saved = (0..3).any(|i| {
        if i > 0 {
            std::thread::sleep(Duration::from_secs(1));
        }
        store.finish_run(run_id, &outcome).map_err(|e| eprintln!("ouro: 답을 못 적었어요 — {e}")).is_ok()
    });
    lock(stops).remove(&run_id);
    on_change();
    match (saved, outcome.status) {
        (false, _) => notify(&title, "답을 저장하지 못했어요 — 디스크를 확인해 주세요"),
        (_, "done") => notify(&title, "답이 왔어요"),
        (_, "failed") => notify(&title, "부탁이 실패했어요 — 팝오버에서 확인해 주세요"),
        _ => {}
    }
}

/// 일꾼 스레드를 띄운다. 이전에 끊긴 «도는 중» 기록은 여기서 닫는다.
pub(crate) fn start(store: Arc<Store>, dir: PathBuf, on_change: OnChange) -> Dispatcher {
    if let Err(e) = store.fail_orphan_runs() {
        eprintln!("ouro: 끊긴 실행을 못 닫았어요 — {e}");
    }
    let (tx, rx) = mpsc::channel::<Msg>();
    let stops: Stops = Arc::default();
    let queued: Arc<Mutex<HashSet<i64>>> = Arc::default();
    let closing = Arc::new(AtomicBool::new(false));
    let hold: Arc<Mutex<bool>> = Arc::default();
    let (st, qd, cl, hd) = (stops.clone(), queued.clone(), closing.clone(), hold.clone());
    std::thread::Builder::new()
        .name("ouro-dispatch".into())
        .spawn(move || {
            let mut manual: Vec<(i64, i64)> = vec![];
            let skip = |d: &Due, late_ms: i64| match decide(&d.late, late_ms) {
                Decision::Run => None,
                Decision::Skip(why) => Some(why),
            };
            loop {
                let now = now_ms();
                while let Ok(m) = rx.try_recv() {
                    if let Msg::RunNow(id, at) = m {
                        manual.push((id, at));
                    }
                }
                let mut held_back = vec![];
                for (id, since) in manual.drain(..) {
                    if cl.load(Ordering::SeqCst) {
                        lock(&qd).remove(&id);
                        continue;
                    }
                    let claimed = {
                        let held = lock(&hd);
                        if *held {
                            // 업데이트 설치 중 — 누른 «지금 실행» 은 버리지 않고 줄에 남긴다(설치가 실패하면 그때 돈다).
                            held_back.push((id, since));
                            continue;
                        }
                        lock(&qd).remove(&id);
                        store.claim_run(id, Claim::Manual { since }, |_, _| None)
                    };
                    if let Ok(Some((run_id, d, _))) = claimed {
                        execute(&store, &dir, &st, &cl, &on_change, run_id, &d);
                    }
                }
                manual = held_back;
                match store.due_errands(now) {
                    Ok(due) => {
                        for d in due {
                            if cl.load(Ordering::SeqCst) {
                                break;
                            }
                            // 하루 상한은 `claim_run` 이 지금 값으로 본다(상한이면 None — 대기로 남는다).
                            // 목록을 읽은 뒤 사람이 고치거나 지웠을 수 있다 — 확보할 때 현재 값으로 다시 읽는다.
                            // 업데이트 설치 중이면 확보하지 않는다 — 잠금을 쥔 채 확인·확보한다(`hold` 주석).
                            let claimed = {
                                let held = lock(&hd);
                                if *held {
                                    break;
                                }
                                store.claim_run(d.id, Claim::Scheduled, skip)
                            };
                            match claimed {
                                Ok(Some((_, cur, true))) => {
                                    on_change();
                                    notify(&title_of(&cur.prompt), "건너뛰었어요 — 예약한 시각을 놓쳤어요");
                                }
                                Ok(Some((run_id, cur, false))) => execute(&store, &dir, &st, &cl, &on_change, run_id, &cur),
                                Ok(None) => {}
                                Err(e) => eprintln!("ouro: 실행 확보 실패 — {e}"),
                            }
                        }
                    }
                    Err(e) => eprintln!("ouro: 부탁 목록 읽기 실패 — {e}"),
                }
                let nap = match store.next_errand_at(now_ms()) {
                    Ok(Some(at)) => Duration::from_millis((at - now_ms()).max(0) as u64).min(MAX_NAP),
                    _ => MAX_NAP,
                };
                match rx.recv_timeout(nap) {
                    Ok(Msg::RunNow(id, at)) => manual.push((id, at)),
                    Ok(Msg::Poke) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("디스패처 스레드를 띄우지 못했어요");
    Dispatcher { tx, stops, queued, closing, hold }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn decide_by_policy() {
        assert_eq!(decide("skip", 0), Decision::Run);
        assert_eq!(decide("skip", SKIP_GRACE_MS), Decision::Run);
        assert!(matches!(decide("skip", SKIP_GRACE_MS + 1), Decision::Skip(_)));
        assert_eq!(decide("run", 3 * 60 * 60 * 1000), Decision::Run, "늦게라도 실행");
        assert!(matches!(decide("run", LATE_RUN_MAX_MS + 1), Decision::Skip(_)), "며칠 묵은 건 «run» 이어도 안 돈다");
    }

    #[test]
    fn args_default_is_conversation_only() {
        let a = claude_args("", None);
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "", "도구 없음");
        assert!(a.contains(&"--strict-mcp-config".to_string()) && !a.contains(&"--allowedTools".to_string()));
        assert!(!a.iter().any(|x| x.contains("prompt")), "문장은 인자가 아니다");
        let w = claude_args("WebSearch", None);
        let j = w.iter().position(|x| x == "--allowedTools").unwrap();
        assert_eq!(w[j + 1], "WebSearch");
    }

    #[test]
    fn parses_success_error_and_garbage() {
        let ok = parse_output(r#"{"type":"result","is_error":false,"result":"안녕","session_id":"s1"}"#, "", Some(0));
        assert_eq!((ok.status, ok.response.as_deref(), ok.session_id.as_deref()), ("done", Some("안녕"), Some("s1")));
        // 이벤트 배열 모양.
        let arr = parse_output(r#"[{"type":"system"},{"type":"result","result":"답","is_error":false}]"#, "", Some(0));
        assert_eq!(arr.response.as_deref(), Some("답"));
        // 로그인 안 됨: 종료코드 1 + JSON 오류. 이유가 stderr 칸에 온다.
        let bad = parse_output(r#"{"type":"result","is_error":true,"result":"Not logged in"}"#, "", Some(1));
        assert_eq!((bad.status, bad.response, bad.stderr.as_deref()), ("failed", None, Some("Not logged in")));
        // 종료코드 0 인데 is_error → 실패. 빈 답도 실패.
        assert_eq!(parse_output(r#"{"is_error":true,"result":"x"}"#, "", Some(0)).status, "failed");
        assert_eq!(parse_output(r#"{"result":"  "}"#, "", Some(0)).status, "failed");
        assert_eq!(parse_output("not json", "boom", Some(2)).stderr.as_deref(), Some("boom"));
    }

    const TEST_PATH: &str = "/usr/bin:/bin";

    fn script(dir: &Path, body: &str) -> PathBuf {
        let p = dir.join("fake-claude");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn runs_a_process_feeds_stdin_and_reads_json() {
        let d = tempfile::tempdir().unwrap();
        // 표준입력으로 받은 문장을 그대로 답에 넣어 돌려준다.
        let exe = script(d.path(), r#"IN=$(cat); printf '{"type":"result","is_error":false,"result":"got:%s","session_id":"abc"}' "$IN""#);
        let stop = AtomicBool::new(false);
        let o = run_process(&exe, &[], "--위험한 시작", d.path(), TEST_PATH, std::process::id(), &stop, Duration::from_secs(10), parse_output);
        assert_eq!((o.status, o.response.as_deref()), ("done", Some("got:--위험한 시작")));
    }

    #[test]
    fn stop_and_timeout_kill_the_whole_group() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "sleep 30 & wait");
        let t = Instant::now();
        let o = run_process(&exe, &[], "x", d.path(), TEST_PATH, std::process::id(), &AtomicBool::new(false), Duration::from_millis(400), parse_output);
        assert_eq!(o.status, "failed");
        assert!(o.stderr.unwrap().contains("멈췄어요") && t.elapsed() < Duration::from_secs(5));

        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            s2.store(true, Ordering::SeqCst);
        });
        let t = Instant::now();
        let o = run_process(&exe, &[], "x", d.path(), TEST_PATH, std::process::id(), &stop, Duration::from_secs(60), parse_output);
        assert_eq!(o.status, "stopped");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn grandchild_holding_the_pipe_does_not_hang_the_worker() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), r#"cat >/dev/null; sleep 30 & printf '{"result":"ok"}'"#);
        let t = Instant::now();
        let o = run_process(&exe, &[], "x", d.path(), TEST_PATH, std::process::id(), &AtomicBool::new(false), Duration::from_secs(60), parse_output);
        assert_eq!(o.response.as_deref(), Some("ok"));
        assert!(t.elapsed() < Duration::from_secs(8), "출력 파이프를 붙든 손주가 일꾼을 막지 않는다");
    }

    #[test]
    fn child_dies_when_the_app_is_killed() {
        // 앱이 강제 종료돼 `shutdown` 이 못 돌아도 감시꾼이 그룹째 치운다 — 가짜 «앱» 을 띄웠다가 죽여 본다.
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "echo $$ > pid; sleep 60 & wait");
        let mut app = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let app_pid = app.id();
        let dir = d.path().to_path_buf();
        let t = Instant::now();
        let worker = std::thread::spawn(move || {
            run_process(&exe, &[], "x", &dir, TEST_PATH, app_pid, &AtomicBool::new(false), Duration::from_secs(60), parse_output)
        });
        std::thread::sleep(Duration::from_millis(500));
        app.kill().unwrap();
        app.wait().unwrap(); // 거두지 않으면 좀비라 `kill -0` 이 계속 성공한다
        let o = worker.join().unwrap();
        assert_eq!(o.status, "failed");
        assert!(t.elapsed() < Duration::from_secs(8), "앱이 죽으면 몇 초 안에 자식도 멈춘다 ({:?})", t.elapsed());
    }

    #[test]
    fn stop_before_spawn_never_starts_the_cli() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "touch started");
        let o = run_process(&exe, &[], "x", d.path(), TEST_PATH, std::process::id(), &AtomicBool::new(true), Duration::from_secs(5), parse_output);
        assert_eq!(o.status, "stopped");
        assert!(!d.path().join("started").exists(), "중단이 먼저면 띄우지 않는다");
    }

    #[test]
    fn child_path_keeps_login_shell_dirs_after_the_cli_dir() {
        let p = child_path(Path::new("/x/bin/claude"), Some("/nvm/bin:/usr/bin"), Some("/usr/bin:/bin"));
        assert_eq!(p, "/x/bin:/nvm/bin:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin");
        assert!(child_path(Path::new("/x/claude"), None, None).starts_with("/x:/opt/homebrew/bin"));
    }

    #[test]
    fn missing_executable_fails_with_a_reason() {
        let d = tempfile::tempdir().unwrap();
        let o = run_process(&d.path().join("nope"), &[], "x", d.path(), TEST_PATH, std::process::id(), &AtomicBool::new(false), Duration::from_secs(1), parse_output);
        assert_eq!(o.status, "failed");
        assert!(o.stderr.unwrap().contains("못 띄웠어요"));
    }

    #[test]
    fn dispatcher_runs_due_errand_once_end_to_end() {
        use crate::errands::ErrandInput;
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), r#"cat >/dev/null; printf '{"type":"result","is_error":false,"result":"끝"}'"#);
        std::env::set_var("OURO_CLAUDE", &exe);
        let store = Arc::new(Store::open_in_memory());
        let e = store
            .create_errand(&ErrandInput { prompt: "브리핑".into(), start_at: now_ms() - 1_000, ..ErrandInput::default() })
            .unwrap();
        let changes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c2 = changes.clone();
        let disp = start(store.clone(), d.path().to_path_buf(), Arc::new(move || {
            c2.fetch_add(1, Ordering::SeqCst);
        }));
        for _ in 0..100 {
            if store.get_errand(e.id).unwrap().unwrap().run.is_some_and(|r| r.status == "done") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let r = store.get_errand(e.id).unwrap().unwrap().run.unwrap();
        assert_eq!((r.status.as_str(), r.response.as_deref(), r.sent_text.as_str()), ("done", Some("끝"), "브리핑"));
        assert!(changes.load(Ordering::SeqCst) >= 2, "시작·끝 두 번 화면에 알린다");
        // 건너뜀 기록이 아닌 한 번 돈 부탁은 예약 시각이 지나도 다시 안 돈다.
        assert!(store.due_errands(now_ms()).unwrap().is_empty());
        drop(disp);
    }

    #[test]
    fn hold_keeps_due_and_manual_errands_from_starting_until_release() {
        // 업데이트 설치(update.rs)가 붙잡은 동안엔 때가 된 부탁도 «지금 실행» 도 확보되지 않는다. 풀면 돈다.
        // 실행 결과(성공·실패)는 보지 않는다 — 다른 테스트가 OURO_CLAUDE 를 바꿔도 «기록이 생겼나» 만 본다.
        use crate::errands::ErrandInput;
        let d = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open_in_memory());
        let disp = start(store.clone(), d.path().to_path_buf(), Arc::new(|| {}));
        disp.hold();
        let due = store
            .create_errand(&ErrandInput { prompt: "때 됨".into(), start_at: now_ms() - 1_000, ..ErrandInput::default() })
            .unwrap();
        let later = store
            .create_errand(&ErrandInput { prompt: "지금".into(), start_at: now_ms() + 3_600_000, ..ErrandInput::default() })
            .unwrap();
        disp.poke();
        assert!(disp.run_now(later.id));
        std::thread::sleep(Duration::from_millis(400));
        let started = |id| store.get_errand(id).unwrap().unwrap().run.is_some();
        assert!(!started(due.id) && !started(later.id), "붙잡힌 동안엔 아무것도 시작하지 않는다");
        assert!(!disp.run_now(later.id), "막힌 «지금 실행» 은 버리지 않고 줄에 남는다");

        disp.release();
        for _ in 0..100 {
            if started(due.id) && started(later.id) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(started(due.id) && started(later.id), "풀면 둘 다 돈다");
        disp.shutdown();
    }

    #[test]
    fn run_now_twice_queues_once() {
        let (tx, rx) = mpsc::channel();
        let d = Dispatcher { tx, stops: Arc::default(), queued: Arc::default(), closing: Arc::default(), hold: Arc::default() };
        assert!(d.run_now(7));
        assert!(!d.run_now(7), "두 번 눌러도 한 번");
        assert!(d.run_now(8));
        assert_eq!(rx.try_iter().count(), 2);
        assert!(!d.stop(1), "도는 게 없으면 false");
    }

    #[test]
    fn codex_args_are_conversation_only_and_resume_safely() {
        let a = codex_args("", None);
        assert_eq!((a[0].as_str(), a.last().unwrap().as_str()), ("exec", "-"), "문장은 표준입력");
        assert!(a.contains(&"--ignore-user-config".to_string()) && a.contains(&"web_search=\"disabled\"".to_string()));
        assert!(a.contains(&"features.shell_tool=false".to_string()) && !a.iter().any(|x| x.starts_with("--disable")));
        let r = codex_args("WebSearch", Some("0199-abc"));
        assert_eq!(&r[..2], ["exec", "resume"]);
        assert_eq!(&r[r.len() - 2..], ["0199-abc", "-"]);
        assert!(r.contains(&"web_search=\"live\"".to_string()));
        let c = claude_args("", Some("s-1"));
        let i = c.iter().position(|x| x == "--resume").unwrap();
        assert_eq!(c[i + 1], "s-1");
        assert!(safe_session("01a1009c-8fef-7571-b4b5-69080a78cd0d"));
        assert!(!safe_session("--dangerously-bypass-approvals-and-sandbox") && !safe_session("a b") && !safe_session(""));
    }

    #[test]
    fn parses_codex_jsonl() {
        // 2026-10-03 실측 모양: 중간 말 + 웹 검색 + 마지막 말. 경고(item 의 error)는 실패가 아니다.
        let ok = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Codex is ignoring 1 unrecognized configuration setting."}}
{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"검색하겠습니다."}}
{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"흐리고 22°C"}}
{"type":"turn.completed","usage":{}}"#;
        let o = parse_codex_output(ok, "", Some(0));
        assert_eq!((o.status, o.response.as_deref(), o.session_id.as_deref()), ("done", Some("흐리고 22°C"), Some("t-1")));
        let bad = r#"{"type":"thread.started","thread_id":"t-2"}
{"type":"error","message":"{\"type\":\"error\",\"status\":400,\"error\":{\"message\":\"model not supported\"}}"}
{"type":"turn.failed","error":{"message":"{\"type\":\"error\",\"status\":400,\"error\":{\"message\":\"model not supported\"}}"}}"#;
        let o = parse_codex_output(bad, "", Some(1));
        assert_eq!((o.status, o.stderr.as_deref()), ("failed", Some("model not supported")));
        assert_eq!(parse_codex_output("", "Not logged in", Some(1)).stderr.as_deref(), Some("Not logged in"));
        assert_eq!(parse_codex_output("", "", Some(0)).status, "failed", "빈 답은 실패");
    }

    #[test]
    fn codex_errand_runs_through_its_own_cli_end_to_end() {
        use crate::errands::ErrandInput;
        let d = tempfile::tempdir().unwrap();
        // 받은 인자 첫 줄과 표준입력을 답에 넣는다 — Codex 갈래로 갔는지 본다.
        let exe = script(
            d.path(),
            r#"IN=$(cat); printf '{"type":"thread.started","thread_id":"t9"}\n{"type":"item.completed","item":{"type":"agent_message","text":"%s:%s"}}\n' "$1" "$IN""#,
        );
        std::env::set_var("OURO_CODEX", &exe);
        let store = Arc::new(Store::open_in_memory());
        let e = store
            .create_errand(&ErrandInput { prompt: "코덱스에게".into(), start_at: now_ms() - 1_000, target: "codex".into(), ..ErrandInput::default() })
            .unwrap();
        let disp = start(store.clone(), d.path().to_path_buf(), Arc::new(|| {}));
        for _ in 0..100 {
            if store.get_errand(e.id).unwrap().unwrap().run.is_some_and(|r| r.status == "done") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let r = store.get_errand(e.id).unwrap().unwrap().run.unwrap();
        assert_eq!((r.status.as_str(), r.response.as_deref(), r.can_resume), ("done", Some("exec:코덱스에게"), true));
        drop(disp);
    }
}
