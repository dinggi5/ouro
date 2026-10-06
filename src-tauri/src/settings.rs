// 설정 창 (개발 16) — iCloud 동기화·실행 맥·아침 브리핑·업데이트·버전을 한곳에.
// 그전엔 메뉴바 우클릭 메뉴(«iCloud 동기화…»·«업데이트 확인…»)와 팝오버 카드에 흩어져 있었다.
//
// 설계 결정:
//   · 같은 React 를 띄우고 프론트가 창 이름 `settings` 를 보고 설정 화면만 그린다(main.tsx).
//   · 크게 보기와 같은 결: 열린 동안만 Regular(도크·⌘Tab), 닫으면 창을 없앤다. 도크 정책은 `wide::policy_after`
//     가 «아직 남은 보통 창» 을 보고 정한다 — 둘 중 하나를 닫았다고 다른 하나가 도크에서 사라지면 안 된다.
//   · 크기 고정. 칸 넷이 한 화면에 들어가는 높이라 스크롤은 노트가 긴 업데이트를 펼쳤을 때만 생긴다.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

pub(crate) const LABEL: &str = "settings";

/// 연다(이미 있으면 앞으로). 팝오버는 물러난다 — 설정 창이 팝오버 뒤에 깔리지 않게.
pub(crate) fn open<R: Runtime>(app: &AppHandle<R>) {
    crate::tray::hide(app);
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("설정")
        .inner_size(480.0, 620.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .build();
    match built {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => {
            eprintln!("ouro: 설정 창을 못 열었어요 — {e}");
            crate::wide::policy_after(app, LABEL);
        }
    }
}
