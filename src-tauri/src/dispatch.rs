// 디스패처 — 때가 된 부탁을 `claude -p` 로 보내고, 답을 받아 `runs` 에 적는다(PLAN §9).
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
//   · 시간 제한 10분, 하루 실행 상한 30(예약 실행만 — 사람이 누른 «지금 실행» 은 세지만 막지 않는다). 프로세스 그룹째 멈춘다.
//   · 앱이 꺼진 채 남은 «도는 중» 은 켤 때 «끊김» 으로 닫는다 — 프로세스가 이미 없다.
//   · `claude` 는 Finder 로 켠 앱의 좁은 PATH 에선 안 보인다 — 흔한 설치 자리, 안 되면 로그인 셸에 물어 찾는다(개발 5 첫 확인 거리).

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
use crate::errands::{title_of, Due, Outcome};
use crate::store::{self, now_ms, Store};

const MAX_NAP: Duration = Duration::from_secs(30);
const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SKIP_GRACE_MS: i64 = 5 * 60 * 1000;
const LATE_RUN_MAX_MS: i64 = 12 * 60 * 60 * 1000;
const DAILY_CAP: i64 = 30;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// 읽어 둘 출력 상한(바이트). 넘는 건 버리되 계속 비워 준다(안 비우면 자식이 파이프에 막힌다).
const OUTPUT_CAP: usize = 2 * 1024 * 1024;

pub(crate) type OnChange = Arc<dyn Fn() + Send + Sync>;

enum Msg {
    Poke,
    RunNow(i64),
}

type Stops = Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>;

/// 디스패처 손잡이.
pub(crate) struct Dispatcher {
    tx: Sender<Msg>,
    stops: Stops,
    /// «지금 실행» 으로 줄에 선 부탁 — 두 번 눌러도 한 번만.
    queued: Arc<Mutex<HashSet<i64>>>,
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
        let _ = self.tx.send(Msg::RunNow(id));
        true
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

/// `claude` 에 줄 인자 (순수). 부탁 문장은 표준입력으로 간다.
fn claude_args(allowed_tools: &str) -> Vec<String> {
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
    a
}

/// `claude` 를 찾는다. 디버그 빌드에선 `OURO_CLAUDE` 로 바꿀 수 있다(테스트용 가짜 실행 파일).
fn find_claude() -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    if let Some(p) = std::env::var_os("OURO_CLAUDE") {
        return Ok(PathBuf::from(p));
    }
    let home = dirs::home_dir();
    let mut cands: Vec<PathBuf> = vec![];
    if let Some(h) = &home {
        for rel in [".local/bin/claude", ".claude/local/claude", ".npm-global/bin/claude", ".bun/bin/claude"] {
            cands.push(h.join(rel));
        }
    }
    cands.extend(["/opt/homebrew/bin/claude", "/usr/local/bin/claude"].map(PathBuf::from));
    if let Some(p) = cands.into_iter().find(|p| p.is_file()) {
        return Ok(p);
    }
    // 로그인 셸의 PATH — nvm 같은 곳에 깔았을 때. 대화형(-i)은 rc 파일 잡음이 섞이니 마지막 «/» 로 시작하는 줄만 쓴다.
    let out = Command::new("/bin/zsh")
        .args(["-lic", "command -v claude"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("Claude Code 를 찾는 중 셸을 못 띄웠어요: {e}"))?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('/') && Path::new(l).is_file())
        .map(PathBuf::from)
        .ok_or_else(|| "Claude Code(`claude`)를 못 찾았어요 — 설치돼 있고 로그인돼 있어야 해요".to_string())
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

fn drain(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let (mut buf, mut chunk) = (Vec::new(), [0u8; 8192]);
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let room = OUTPUT_CAP.saturating_sub(buf.len());
            buf.extend_from_slice(&chunk[..n.min(room)]);
        }
        buf
    })
}

fn kill_group(pid: u32) {
    // 프로세스 그룹 전체 — `claude` 가 띄운 자식(검색 도구 등)까지.
    unsafe {
        libc::killpg(pid as i32, libc::SIGKILL);
    }
}

/// 프로세스를 돌려 끝날 때까지 기다린다. `stop` 이 켜지면 멈추고(stopped), `timeout` 을 넘으면 멈춘다(failed).
fn run_process(exe: &Path, args: &[String], prompt: &str, cwd: &Path, stop: &AtomicBool, timeout: Duration) -> Outcome {
    let fail = |msg: String| Outcome { status: "failed", exit_code: None, response: None, stderr: Some(msg), session_id: None };
    let path_env = format!(
        "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin",
        exe.parent().map(|p| p.display().to_string()).unwrap_or_default()
    );
    let mut child = match Command::new(exe)
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
        Err(e) => return fail(format!("`claude` 를 못 띄웠어요: {e}")),
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
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    if stopped {
        return Outcome { status: "stopped", exit_code: None, response: None, stderr: None, session_id: None };
    }
    if timed_out {
        return fail(format!("{}분을 넘겨 멈췄어요", timeout.as_secs() / 60));
    }
    parse_output(&stdout, &stderr, status.and_then(|s| s.code()))
}

fn notify(title: &str, body: &str) {
    show_notification(title, body, || {});
}

/// 부탁 하나를 돌린다. `late_ms` = 예약보다 늦은 정도(수동 실행은 0).
fn execute(store: &Store, dir: &Path, stops: &Stops, on_change: &OnChange, d: &Due, late_ms: i64) {
    let title = title_of(&d.prompt);
    let run_id = match store.begin_run(d.id, &d.prompt, late_ms, None) {
        Ok(id) => id,
        Err(e) => return eprintln!("ouro: 실행 기록을 못 열었어요 — {e}"),
    };
    let flag = Arc::new(AtomicBool::new(false));
    lock(stops).insert(run_id, flag.clone());
    on_change();

    let outcome = match (find_claude(), store::create_private_dir(&dir.join("runs").join(d.id.to_string()))) {
        (Err(e), _) | (_, Err(e)) => Outcome { status: "failed", exit_code: None, response: None, stderr: Some(e), session_id: None },
        (Ok(exe), Ok(())) => {
            run_process(&exe, &claude_args(&d.allowed_tools), &d.prompt, &dir.join("runs").join(d.id.to_string()), &flag, RUN_TIMEOUT)
        }
    };
    if let Err(e) = store.finish_run(run_id, &outcome) {
        eprintln!("ouro: 답을 못 적었어요 — {e}");
    }
    lock(stops).remove(&run_id);
    on_change();
    match outcome.status {
        "done" => notify(&title, "답이 왔어요"),
        "failed" => notify(&title, "부탁이 실패했어요 — 팝오버에서 확인해 주세요"),
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
    let (st, qd) = (stops.clone(), queued.clone());
    std::thread::Builder::new()
        .name("ouro-dispatch".into())
        .spawn(move || {
            let mut manual: Vec<i64> = vec![];
            loop {
                let now = now_ms();
                while let Ok(m) = rx.try_recv() {
                    if let Msg::RunNow(id) = m {
                        manual.push(id);
                    }
                }
                for id in manual.drain(..) {
                    lock(&qd).remove(&id);
                    if let (Ok(false), Ok(Some(d))) = (store.is_running(id), store.errand_for_run(id)) {
                        execute(&store, &dir, &st, &on_change, &d, 0);
                    }
                }
                match store.due_errands(now) {
                    Ok(due) => {
                        for d in due {
                            let late_ms = (now_ms() - d.start_at).max(0);
                            match decide(&d.late, late_ms) {
                                Decision::Skip(why) => {
                                    if store.begin_run(d.id, &d.prompt, late_ms, Some(why)).is_ok() {
                                        on_change();
                                        notify(&title_of(&d.prompt), "건너뛰었어요 — 예약한 시각을 놓쳤어요");
                                    }
                                }
                                Decision::Run => {
                                    if store.runs_since(now_ms() - DAY_MS).unwrap_or(0) >= DAILY_CAP {
                                        break; // 상한 — 대기로 남겨 두고 다음 바퀴에 다시 본다
                                    }
                                    execute(&store, &dir, &st, &on_change, &d, late_ms);
                                }
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
                    Ok(Msg::RunNow(id)) => manual.push(id),
                    Ok(Msg::Poke) | Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("디스패처 스레드를 띄우지 못했어요");
    Dispatcher { tx, stops, queued }
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
        let a = claude_args("");
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "", "도구 없음");
        assert!(a.contains(&"--strict-mcp-config".to_string()) && !a.contains(&"--allowedTools".to_string()));
        assert!(!a.iter().any(|x| x.contains("prompt")), "문장은 인자가 아니다");
        let w = claude_args("WebSearch");
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
        let o = run_process(&exe, &[], "--위험한 시작", d.path(), &stop, Duration::from_secs(10));
        assert_eq!((o.status, o.response.as_deref()), ("done", Some("got:--위험한 시작")));
    }

    #[test]
    fn stop_and_timeout_kill_the_whole_group() {
        let d = tempfile::tempdir().unwrap();
        let exe = script(d.path(), "sleep 30 & wait");
        let t = Instant::now();
        let o = run_process(&exe, &[], "x", d.path(), &AtomicBool::new(false), Duration::from_millis(400));
        assert_eq!(o.status, "failed");
        assert!(o.stderr.unwrap().contains("멈췄어요") && t.elapsed() < Duration::from_secs(5));

        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            s2.store(true, Ordering::SeqCst);
        });
        let t = Instant::now();
        let o = run_process(&exe, &[], "x", d.path(), &stop, Duration::from_secs(60));
        assert_eq!(o.status, "stopped");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_executable_fails_with_a_reason() {
        let d = tempfile::tempdir().unwrap();
        let o = run_process(&d.path().join("nope"), &[], "x", d.path(), &AtomicBool::new(false), Duration::from_secs(1));
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
            .create_errand(&ErrandInput { prompt: "브리핑".into(), start_at: now_ms() - 1_000, allowed_tools: String::new(), late: "run".into() })
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
    fn run_now_twice_queues_once() {
        let (tx, rx) = mpsc::channel();
        let d = Dispatcher { tx, stops: Arc::default(), queued: Arc::default() };
        assert!(d.run_now(7));
        assert!(!d.run_now(7), "두 번 눌러도 한 번");
        assert!(d.run_now(8));
        assert_eq!(rx.try_iter().count(), 2);
        assert!(!d.stop(1), "도는 게 없으면 false");
    }
}
