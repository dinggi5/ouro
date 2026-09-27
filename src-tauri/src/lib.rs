// Ouro 코어 — 선순환 캘린더. (한글 별칭: 우로우로)
//
// 모듈 지도:
//   tray   메뉴바 상주 — 트레이 아이콘 + 팝오버 위치·자동 숨김 (Kura 에서 이식, 개발 1)
//
// 이 파일에는 앱 셸만 둔다: 창 제어 커맨드 + run(). DB·스케줄러·소켓은 개발 2~ 에서 모듈로 붙는다.

mod tray;

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
        .invoke_handler(tauri::generate_handler![hide_popover])
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
