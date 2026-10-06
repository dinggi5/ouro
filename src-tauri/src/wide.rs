// «크게 보기» 창 (개발 12) — 같은 앱의 두 번째 창. 팝오버(main)와 같은 React 를 넓은 배치로 띄운다
// (프론트는 창 이름 `wide` 를 보고 세 칸으로 그린다 — App.tsx).
//
// 설계 결정:
//   · 팝오버가 주인공이다. 이 창은 팝오버의 «펼치기»·메뉴바 우클릭 «크게 보기» 로만 열고, 켤 때 저절로 뜨지 않는다.
//   · 이 창이 사는 동안만 Regular(도크·⌘Tab 에 보인다), 닫히면(Destroyed) Accessory 로 — Fantastical 과 같은 구조.
//     닫기 = 창을 없앤다(숨기지 않는다). 다시 열 때 새로 읽는 데 1ms 라 붙들 이유가 없고, 숨긴 창이 남아 있으면
//     도크 정책을 언제 되돌릴지가 애매해진다.
//   · 팝오버의 blur-숨김·닫기-숨김은 이 창에 걸지 않는다(lib.rs on_window_event 가 창 이름으로 가른다).

use tauri::{AppHandle, LogicalPosition, Manager, Runtime, TitleBarStyle, WebviewUrl, WebviewWindowBuilder};

pub(crate) const LABEL: &str = "wide";

/// 연다(이미 있으면 앞으로). 팝오버는 물러난다 — 같은 화면이 두 곳에 떠 있을 이유가 없다.
pub(crate) fn open<R: Runtime>(app: &AppHandle<R>) {
    crate::tray::hide(app);
    // 창보다 먼저 Regular — 창이 뜬 뒤에 바꾸면 앱이 한 번 비활성화됐다 돌아오며 포커스가 튄다.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Ouro")
        // 최소 = 왼쪽 288 + 오른쪽 360 + 가운데 팝오버 폭(420)에 가깝게 — 더 좁으면 가운데 머리 버튼이 제목을 덮는다(코덱스 개발 12).
        .inner_size(1180.0, 760.0)
        .min_inner_size(1060.0, 600.0)
        // 제목 막대를 화면에 녹인다 — 신호등만 남기고 세 칸이 창 맨 위까지 올라간다(드래그는 data-tauri-drag-region).
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(LogicalPosition::new(20.0, 22.0))
        .center()
        .build();
    match built {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => {
            eprintln!("ouro: 크게 보기 창을 못 열었어요 — {e}");
            policy_after(app, LABEL);
        }
    }
}

/// 보통 창(크게 보기·설정) 하나가 없어졌을 때 — 남은 보통 창이 없으면 다시 메뉴바에만 산다(개발 16: 설정 창이 생겨 둘이 됐다).
/// `gone` 은 지금 없어지는 창 — Destroyed 시점엔 아직 목록에 남아 있을 수 있어 빼고 센다.
pub(crate) fn policy_after<R: Runtime>(app: &AppHandle<R>, gone: &str) {
    let left = [LABEL, crate::settings::LABEL].into_iter().any(|l| l != gone && app.get_webview_window(l).is_some());
    #[cfg(target_os = "macos")]
    if !left {
        let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = left;
}

/// 보통 창(크게 보기 먼저, 없으면 설정)이 열려 있으면 앞으로 가져오고 true. 도크 아이콘 클릭(Reopen)이 팝오버 대신 이 창으로 가게.
pub(crate) fn raise<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(w) = app.get_webview_window(LABEL).or_else(|| app.get_webview_window(crate::settings::LABEL)) else {
        return false;
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
    true
}

/// 사람이 지금 이 창을 보고 있나 — 새 AI 제안이 와도 팝오버를 겹쳐 띄우지 않으려고(이 창도 제안 카드를 보인다).
pub(crate) fn is_front<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window(LABEL)
        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
        .unwrap_or(false)
}
