// Ouro 코어 — 선순환 캘린더. (한글 별칭: 우로우로)
//
// 모듈 지도:
//   tray    메뉴바 상주 — 트레이 아이콘 + 팝오버 위치·자동 숨김 (Kura 에서 이식, 개발 1)
//   store   ~/.ouro/ouro.db — 일정·부탁·실행 (개발 2)
//   alerts  일정 알림 + 매일 백업을 도는 시계 스레드 (개발 2)
//
// 이 파일에는 앱 셸만 둔다: 커맨드(프론트가 부르는 문) + run(). 판단은 전부 모듈에 있다.

mod alerts;
mod store;
mod tray;

use std::sync::Arc;

use store::{Event, EventInput, Store};
use tauri::State;

/// 앱 코어. DB 를 못 열었으면 Err 를 그대로 들고 산다 — 앱이 켜지다 죽으면 사용자는 «메뉴바에 아무것도 없다» 만 보지만,
/// 이렇게 두면 팝오버가 뜨고 무엇이 잘못됐는지 한 줄로 보여 줄 수 있다.
struct Core {
    store: Arc<Store>,
    alerts: alerts::Alerts,
}

type CoreState = Result<Core, String>;

fn core<'a>(state: &'a State<'_, CoreState>) -> Result<&'a Core, String> {
    state.inner().as_ref().map_err(Clone::clone)
}

fn open_core() -> CoreState {
    let dir = store::data_dir().ok_or("홈 폴더를 찾지 못했어요")?;
    let store = Arc::new(Store::open(&dir)?);
    let alerts = alerts::start(store.clone(), dir.join("backup"));
    Ok(Core { store, alerts })
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

/// ⌘W 로 팝오버를 닫는다. 창이 테두리 없음이라 ⌘W 가 러스트의 CloseRequested 까지 오지 않아
/// 프론트가 직접 부른다(App.tsx). 종착지는 트레이 클릭과 같은 `tray::hide`.
#[tauri::command]
fn hide_popover(app: tauri::AppHandle) {
    tray::hide(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(tray::PopoverState::default())
        .manage(open_core())
        .setup(|app| {
            // 도크 아이콘 없이 메뉴바에만 산다(tray.rs 머리 주석). 정본은 Info.plist 의 LSUIElement 다 —
            // 🔴 이 줄만으로 하면 앱이 Regular 로 켜졌다 Accessory 로 바뀌며 **비활성화**되고, 그 순간
            // 팝오버가 blur 로 곧장 숨는다(개발 1 실측: 켜자마자 뜬 팝오버가 안 보였다). 이 줄은
            // Info.plist 가 안 붙는 경로(`cargo run` 맨 바이너리)를 위한 보험이다.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            tray::build(app.handle())?;
            // 켤 때마다 한 번 띄운다. 도크 아이콘이 없어서, 메뉴바가 꽉 차 트레이가 노치 뒤로
            // 숨으면 켜도 아무 일도 안 일어난 것처럼 보인다 — 적어도 켠 순간엔 보이게.
            // (로그인 자동 시작이 붙으면 그 경로에선 조용히 뜨게 갈라야 한다.)
            tray::show_at_launch(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 창 닫기는 종료가 아니라 숨기기 — 메뉴바에 계속 산다. 완전 종료는 ⌘Q 또는 트레이 메뉴 «종료».
            tauri::WindowEvent::CloseRequested { api, .. } => {
                use tauri::Manager;
                api.prevent_close();
                tray::hide(window.app_handle());
            }
            tauri::WindowEvent::Focused(false) => tray::on_blur(window),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            hide_popover,
            list_events,
            create_event,
            update_event,
            delete_event,
            restore_event,
            backup_error
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // 도크 아이콘이 없어도 Finder·Spotlight 로 이미 켜진 앱을 다시 열면 이 이벤트가 온다
            // (applicationShouldHandleReopen) — 트레이가 노치 뒤에 숨었을 때의 두 번째 길.
            if let tauri::RunEvent::Reopen { .. } = event {
                tray::show(app);
            }
        });
}
