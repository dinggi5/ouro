// Ouro 코어 — 선순환 캘린더. (한글 별칭: 우로우로)
//
// 모듈 지도:
//   tray    메뉴바 상주 — 트레이 아이콘 + 팝오버 위치·자동 숨김 (Kura 에서 이식, 개발 1)
//   store   ~/.ouro/ouro.db — 일정·부탁·실행 (개발 2)
//   alerts  일정 알림 + 매일 백업 + 아침 브리핑을 도는 시계 스레드 (개발 2·7)
//   brief   오늘 일정·겹침·빈 시간을 규칙으로 센다 (개발 7)
//   parse   빠른 입력 규칙 파서 «내일 3시 치과» → 초안 (개발 3)
//   errands 부탁 저장·실행 기록 (개발 5)
//   dispatch 때가 된 부탁을 `claude -p` 로 보내고 답을 적는 일꾼 (개발 5)
//   mcp     MCP 사이드카가 묻는 소켓 — 읽기·제안만, 승인은 팝오버에서 (개발 4)
//   update  인앱 업데이트 — 확인은 저절로, 설치는 사람이 노트를 보고 누를 때만 (개발 8)
//   sync    iCloud 동기화의 앱 쪽 — 바뀐 줄 장부·레코드 변환·합치기·헬퍼(OuroSync.app) 관리·실행 맥 (개발 10)
//
// 이 파일에는 앱 셸만 둔다: 커맨드(프론트가 부르는 문) + run(). 판단은 전부 모듈에 있다.

mod alerts;
mod brief;
mod dispatch;
mod errands;
mod mcp;
mod notify;
mod parse;
mod store;
mod sync;
mod tray;
mod update;

use std::sync::Arc;

use errands::{Errand, ErrandInput, ErrandProposal};
use parse::{Draft, Engine, Miss};
use serde::Serialize;
use store::{Event, EventInput, Proposal, Store};
use tauri::{Emitter, Manager, State};

/// 앱 코어. DB 를 못 열었으면 Err 를 그대로 들고 산다 — 앱이 켜지다 죽으면 사용자는 «메뉴바에 아무것도 없다» 만 보지만,
/// 이렇게 두면 팝오버가 뜨고 무엇이 잘못됐는지 한 줄로 보여 줄 수 있다.
pub(crate) struct Core {
    pub(crate) store: Arc<Store>,
    presence: Arc<mcp::Presence>,
    alerts: alerts::Alerts,
    pub(crate) dispatcher: dispatch::Dispatcher,
    sync: sync::Syncer,
    dir: std::path::PathBuf,
    /// 한 데이터 폴더에 앱 하나만 — 프로세스가 사는 동안 쥔다.
    _instance: std::fs::File,
}

pub(crate) type CoreState = Result<Core, String>;

fn core<'a>(state: &'a State<'_, CoreState>) -> Result<&'a Core, String> {
    state.inner().as_ref().map_err(Clone::clone)
}

/// 같은 데이터 폴더를 쓰는 앱이 둘 뜨면 디스패처가 둘이라 같은 부탁이 두 번 돌고, 한쪽이 다른 쪽의 실행을 «끊김» 으로 닫는다
/// (코덱스 개발 5). DB 를 열기 **전에** 배타 잠금을 잡는다. 프로세스가 죽으면 OS 가 풀어 준다.
fn lock_instance(dir: &std::path::Path) -> Result<std::fs::File, String> {
    use std::os::unix::io::AsRawFd;
    store::create_private_dir(dir)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("ouro.lock"))
        .map_err(|e| format!("잠금 파일을 못 열었어요: {e}"))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("다른 Ouro 가 이미 떠 있어요 — 메뉴바의 ◯ 를 눌러 주세요".into());
    }
    Ok(f)
}

fn open_core(app: &tauri::AppHandle) -> CoreState {
    let dir = store::data_dir().ok_or("홈 폴더를 찾지 못했어요")?;
    let _instance = lock_instance(&dir)?;
    let store = Arc::new(Store::open(&dir)?);
    let alerts = alerts::start(store.clone(), dir.join("backup"));
    let handle = app.clone();
    let dispatcher = dispatch::start(
        store.clone(),
        dir.clone(),
        Arc::new(move || {
            let _ = handle.emit("errands-changed", ());
        }),
    );
    let handle = app.clone();
    let sync = sync::Syncer::start(
        store.clone(),
        &dir,
        sync::find_helper(),
        sync::computer_name(),
        Arc::new(move |c| {
            if c == sync::Change::Data {
                // 다른 기기에서 온 일정·부탁·제안 — 목록을 다시 읽고, 새로 온 부탁이 있을 수 있으니 디스패처를 깨운다.
                let _ = handle.emit("errands-changed", ());
                let _ = handle.emit("proposals-changed", ());
                if let Some(Ok(core)) = handle.try_state::<CoreState>().map(|s| s.inner().as_ref()) {
                    core.dispatcher.poke();
                }
            }
            let _ = handle.emit("sync-changed", ());
        }),
    );
    Ok(Core { store, alerts, dispatcher, sync, dir, presence: Arc::default(), _instance })
}

/// iCloud 동기화 상태(팝오버 아래 한 줄·설정 카드).
#[tauri::command]
fn sync_status(state: State<'_, CoreState>) -> Result<sync::SyncStatus, String> {
    Ok(core(&state)?.sync.status())
}

#[tauri::command]
fn set_sync(state: State<'_, CoreState>, on: bool) -> Result<sync::SyncStatus, String> {
    core(&state)?.sync.set_enabled(on)
}

/// 이 맥을 «실행 맥» 으로 — 예약 부탁을 이 맥이 돌린다.
#[tauri::command]
fn make_runner(state: State<'_, CoreState>) -> Result<sync::SyncStatus, String> {
    core(&state)?.sync.make_runner()
}

/// 팝오버를 열 때 — 다른 기기에서 바뀐 걸 바로 받는다(푸시를 못 받았을 때의 보험).
#[tauri::command]
fn sync_fetch(state: State<'_, CoreState>) -> Result<(), String> {
    core(&state)?.sync.fetch_now();
    Ok(())
}

/// 부탁 — 예약 시각이 창 [from, to) 인 것과 각자의 최근 실행(상태·답).
#[tauri::command]
fn list_errands(state: State<'_, CoreState>, from: i64, to: i64) -> Result<Vec<Errand>, String> {
    core(&state)?.store.list_errands(from, to)
}

/// 사람이 쓴 부탁을 만든다. 🔴 문장은 사람이 쓴 것(AI 답·일정 글이 아니다) — AI 가 낸 부탁은 `approve_errand_proposal` 로만 들어온다.
#[tauri::command]
fn create_errand(state: State<'_, CoreState>, input: ErrandInput) -> Result<Errand, String> {
    let c = core(&state)?;
    let e = c.store.create_errand(&input)?;
    c.dispatcher.poke();
    Ok(e)
}

#[tauri::command]
fn update_errand(state: State<'_, CoreState>, id: i64, input: ErrandInput) -> Result<Errand, String> {
    let c = core(&state)?;
    let e = c.store.update_errand(id, &input)?;
    c.dispatcher.poke();
    Ok(e)
}

#[tauri::command]
fn delete_errand(state: State<'_, CoreState>, id: i64) -> Result<(), String> {
    core(&state)?.store.delete_errand(id)
}

#[tauri::command]
fn restore_errand(state: State<'_, CoreState>, id: i64) -> Result<Errand, String> {
    let c = core(&state)?;
    let e = c.store.restore_errand(id)?;
    c.dispatcher.poke();
    Ok(e)
}

/// 지금 한 번 돌린다(예약과 무관 · 다시 실행).
#[tauri::command]
fn run_errand_now(state: State<'_, CoreState>, id: i64) -> Result<(), String> {
    let c = core(&state)?;
    if c.store.is_running(id)? {
        return Err("이미 도는 중이에요".into());
    }
    if !c.store.errand_runnable(id)? {
        return Err("없는 부탁이거나 아직 승인 전이에요".into());
    }
    if !c.dispatcher.run_now(id) {
        return Err("이미 줄을 서 있어요".into());
    }
    Ok(())
}

#[tauri::command]
fn stop_run(state: State<'_, CoreState>, run_id: i64) -> Result<(), String> {
    if core(&state)?.dispatcher.stop(run_id) {
        Ok(())
    } else {
        Err("도는 중인 실행이 아니에요".into())
    }
}

/// 한 부탁의 실행 기록(최근 것부터) — 답 시트의 «지난 실행».
#[tauri::command]
fn list_runs(state: State<'_, CoreState>, id: i64) -> Result<Vec<errands::RunView>, String> {
    core(&state)?.store.list_runs(id)
}

#[tauri::command]
fn mark_run_read(state: State<'_, CoreState>, run_id: i64) -> Result<(), String> {
    core(&state)?.store.mark_run_read(run_id)
}

/// 오늘 브리핑(로컬 계산 — 바깥으로 안 나간다).
#[tauri::command]
fn briefing(state: State<'_, CoreState>) -> Result<brief::Briefing, String> {
    brief::today(&core(&state)?.store, store::now_ms())
}

/// 아침 브리핑 알림 켬/끔.
#[tauri::command]
fn set_morning_briefing(state: State<'_, CoreState>, on: bool) -> Result<(), String> {
    core(&state)?.store.set_setting(brief::KEY_ON, if on { "on" } else { "off" })
}

/// 창 [from, to) (UTC ms) 에 걸치는 일정.
#[tauri::command]
fn list_events(state: State<'_, CoreState>, from: i64, to: i64) -> Result<Vec<Event>, String> {
    core(&state)?.store.list_events(from, to)
}

#[tauri::command]
fn create_event(state: State<'_, CoreState>, input: EventInput) -> Result<Event, String> {
    let c = core(&state)?;
    let e = c.store.create_event(&input)?;
    c.alerts.poke();
    Ok(e)
}

#[tauri::command]
fn update_event(state: State<'_, CoreState>, id: i64, input: EventInput) -> Result<Event, String> {
    let c = core(&state)?;
    let e = c.store.update_event(id, &input)?;
    c.alerts.poke();
    Ok(e)
}

#[tauri::command]
fn delete_event(state: State<'_, CoreState>, id: i64) -> Result<(), String> {
    core(&state)?.store.delete_event(id)
}

/// 빠른 입력 한 줄 → 초안. 저장하지 않는다(프론트가 카드로 보여 주고, 확정하면 `create_event` 로 온다).
/// `base` = 사람이 보고 있는 날 `YYYY-MM-DD`(오늘이 아니면) — 날짜 없는 «3시 치과» 가 그날로 간다.
/// DB 를 못 열었어도 돈다(파싱은 저장소와 무관).
#[tauri::command]
fn parse_quick(text: String, base: Option<String>) -> Draft {
    let base = base.and_then(|b| chrono::NaiveDate::parse_from_str(&b, "%Y-%m-%d").ok());
    parse::Rules.parse(&text, chrono::Local::now().naive_local(), base)
}

/// 못 알아들은 입력을 확정했을 때 유형 하나를 센다(PLAN §7 — 본문은 안 남긴다).
#[tauri::command]
fn record_parse_miss(state: State<'_, CoreState>, kind: Miss) -> Result<(), String> {
    core(&state)?.store.record_parse_miss(kind.key())
}

/// 팝오버 아래에 띄울 경고 — 지금은 백업 실패 하나. 없으면 None.
#[tauri::command]
fn backup_error(state: State<'_, CoreState>) -> Result<Option<String>, String> {
    Ok(core(&state)?.alerts.backup_error())
}

#[tauri::command]
fn restore_event(state: State<'_, CoreState>, id: i64) -> Result<Event, String> {
    let c = core(&state)?;
    let e = c.store.restore_event(id)?;
    c.alerts.poke();
    Ok(e)
}

/// 팝오버 카드로 가는 제안 — 겹치는 일정 이름까지.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProposalCard {
    #[serde(flatten)]
    proposal: Proposal,
    conflicts: Vec<String>,
}

#[tauri::command]
fn list_proposals(state: State<'_, CoreState>) -> Result<Vec<ProposalCard>, String> {
    let s = &core(&state)?.store;
    s.pending_proposals()?
        .into_iter()
        .map(|p| Ok(ProposalCard { conflicts: mcp::conflicts(s, &p.event, None)?, proposal: p }))
        .collect()
}

/// 사람이 제안을 받는다(팝오버의 «넣기»·«고쳐서 넣기»). 🔴 제안이 일정이 되는 **유일한** 길 — 소켓엔 이 문이 없다(mcp.rs).
#[tauri::command]
fn approve_proposal(state: State<'_, CoreState>, id: i64, input: Option<EventInput>) -> Result<Event, String> {
    let c = core(&state)?;
    let e = c.store.approve_proposal(id, input.as_ref())?;
    c.alerts.poke();
    Ok(e)
}

#[tauri::command]
fn reject_proposal(state: State<'_, CoreState>, id: i64) -> Result<(), String> {
    core(&state)?.store.reject_proposal(id)
}

#[tauri::command]
fn list_errand_proposals(state: State<'_, CoreState>) -> Result<Vec<ErrandProposal>, String> {
    core(&state)?.store.pending_errand_proposals()
}

/// 사람이 부탁 제안을 승인한다(팝오버의 «승인»·«고쳐서 승인»). 🔴 AI 가 낸 글이 부탁이 되는 **유일한** 길 — 소켓엔 이 문이 없다(mcp.rs).
/// 승인된 부탁은 때가 되면 돈다 — 디스패처를 깨워 다음 예약 시각을 다시 보게 한다.
#[tauri::command]
fn approve_errand_proposal(state: State<'_, CoreState>, id: i64, input: Option<ErrandInput>) -> Result<Errand, String> {
    let c = core(&state)?;
    let e = c.store.approve_errand_proposal(id, input.as_ref())?;
    c.dispatcher.poke();
    Ok(e)
}

#[tauri::command]
fn reject_errand_proposal(state: State<'_, CoreState>, id: i64) -> Result<(), String> {
    core(&state)?.store.reject_errand_proposal(id)
}

/// 소켓을 연다. 새 제안이 오면 팝오버를 띄우고 화면에 알린다(떠 있던 팝오버는 focus 이벤트가 안 오니 이벤트로).
/// 못 열어도 앱은 돈다 — 캘린더는 쓸 수 있고, MCP 쪽은 «앱을 켜 주세요» 로 답한다.
fn start_socket(app: &tauri::AppHandle) {
    let state = app.state::<CoreState>();
    let Ok(c) = state.inner().as_ref() else { return };
    let handle = app.clone();
    let on_change: mcp::OnChange = Arc::new(move |change| {
        let h = handle.clone();
        let _ = handle.run_on_main_thread(move || match change {
            mcp::Change::Proposal => {
                tray::show(&h);
                let _ = h.emit("proposals-changed", ());
            }
            mcp::Change::Presence => {
                let _ = h.emit("mcp-changed", ());
            }
        });
    });
    if let Err(e) = mcp::serve(c.store.clone(), c.presence.clone(), &c.dir, on_change) {
        eprintln!("ouro: MCP 소켓을 못 열었어요 — {e}");
    }
}

/// 지금 붙어 있는 AI(MCP 클라이언트 이름). 팝오버 아래 «연결됨» 줄.
#[tauri::command]
fn mcp_clients(state: State<'_, CoreState>) -> Result<Vec<String>, String> {
    Ok(core(&state)?.presence.clients(std::time::Instant::now()))
}

/// ⌘W 로 팝오버를 닫는다. 창이 테두리 없음이라 ⌘W 가 러스트의 CloseRequested 까지 오지 않아
/// 프론트가 직접 부른다(App.tsx). 종착지는 트레이 클릭과 같은 `tray::hide`.
#[tauri::command]
fn hide_popover(app: tauri::AppHandle) {
    tray::hide(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(tray::PopoverState::default())
        .manage(update::PendingUpdate::default())
        .setup(|app| {
            app.manage(open_core(app.handle()));
            // 도크 아이콘 없이 메뉴바에만 산다(tray.rs 머리 주석). 정본은 Info.plist 의 LSUIElement 다 —
            // 🔴 이 줄만으로 하면 앱이 Regular 로 켜졌다 Accessory 로 바뀌며 **비활성화**되고, 그 순간
            // 팝오버가 blur 로 곧장 숨는다(개발 1 실측: 켜자마자 뜬 팝오버가 안 보였다). 이 줄은
            // Info.plist 가 안 붙는 경로(`cargo run` 맨 바이너리)를 위한 보험이다.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            tray::build(app.handle())?;
            // 알림을 누르면 팝오버를 연다(notify.rs). 권한은 여기서 한 번 묻는다.
            let h = app.handle().clone();
            notify::setup(move || {
                let h2 = h.clone();
                let _ = h.run_on_main_thread(move || tray::show(&h2));
            });
            start_socket(app.handle());
            // 켤 때마다 한 번 띄운다. 도크 아이콘이 없어서, 메뉴바가 꽉 차 트레이가 노치 뒤로
            // 숨으면 켜도 아무 일도 안 일어난 것처럼 보인다 — 적어도 켠 순간엔 보이게.
            // (로그인 자동 시작이 붙으면 그 경로에선 조용히 뜨게 갈라야 한다.)
            tray::show_at_launch(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 창 닫기는 종료가 아니라 숨기기 — 메뉴바에 계속 산다. 완전 종료는 ⌘Q 또는 트레이 메뉴 «종료».
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                tray::hide(window.app_handle());
            }
            tauri::WindowEvent::Focused(false) => tray::on_blur(window),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            hide_popover,
            list_events,
            list_errands,
            create_errand,
            update_errand,
            delete_errand,
            restore_errand,
            run_errand_now,
            stop_run,
            mark_run_read,
            list_runs,
            briefing,
            set_morning_briefing,
            create_event,
            update_event,
            delete_event,
            restore_event,
            backup_error,
            parse_quick,
            record_parse_miss,
            list_proposals,
            approve_proposal,
            reject_proposal,
            list_errand_proposals,
            approve_errand_proposal,
            reject_errand_proposal,
            mcp_clients,
            sync_status,
            set_sync,
            make_runner,
            sync_fetch,
            update::check_update,
            update::install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // 도크 아이콘이 없어도 Finder·Spotlight 로 이미 켜진 앱을 다시 열면 이 이벤트가 온다
            // (applicationShouldHandleReopen) — 트레이가 노치 뒤에 숨었을 때의 두 번째 길.
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show(app);
            }
            // 끌 때 도는 `claude` 가 앱보다 오래 살지 않게 먼저 멈춘다(코덱스 개발 5).
            if let tauri::RunEvent::Exit = event {
                if let Ok(c) = app.state::<CoreState>().inner().as_ref() {
                    c.dispatcher.shutdown();
                    c.sync.shutdown();
                }
            }
        });
}
