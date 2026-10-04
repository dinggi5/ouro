// 알림 — 서명된 앱 번들이면 UNUserNotificationCenter, 아니면 osascript(개발 9).
//
// 설계 결정:
//   · **정석은 UNUserNotificationCenter** 다. osascript 알림은 «스크립트 편집기» 이름·아이콘으로 뜨고 눌러도 Ouro 가 안 열린다.
//     tauri-plugin-notification(notify-rust)은 옛 NSUserNotificationCenter 라 macOS 26 이 조용히 버린다(Kura notify.rs 실측) — 안 쓴다.
//   · UN 은 **번들 id 가 있는 .app** 에서만 부른다. 맨 바이너리(`cargo run`·테스트)에서 부르면 번들 프록시가 없어 예외로 죽는다.
//   · **허용됐을 때만 UN, 아니면 osascript**: 보내기 전에 권한 상태를 묻는다. 허용 안 된 앱의 `addNotificationRequest` 는 오류 없이
//     조용히 버려지기도 해서(2026-10-04 실측 — Developer ID 로 서명한 디버그 번들이 «허용 안 됨» 인데 완료 블록에 오류가 없었다)
//     오류만 보고 갈라선 알림이 사라진다. 허용인데 거는 데 실패해도 osascript 로 — 최악이 «개발 8 과 같음» 이 되게. osascript 도 실패해야 `on_fail`.
//   · 권한 요청은 켤 때 한 번(`setup`). 처음 한 번 macOS 가 «Ouro 가 알림을 보내려고 합니다» 를 묻는다.
//   · 팝오버가 떠 있어(앱이 앞에) 있어도 배너를 띄운다(`willPresent` → 배너·목록). 알림을 누르면 팝오버를 연다.

use std::sync::OnceLock;

/// 알림을 누르면 부를 것(팝오버 열기) — `setup` 이 한 번 정한다.
static ON_CLICK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// 켤 때 한 번 — 델리게이트를 달고 권한을 묻는다. 번들이 아니면 아무것도 안 한다.
pub(crate) fn setup(on_click: impl Fn() + Send + Sync + 'static) {
    let _ = ON_CLICK.set(Box::new(on_click));
    #[cfg(target_os = "macos")]
    if mac::bundled() {
        mac::setup();
    }
}

/// 알림을 띄운다. 끝내 못 띄우면 `on_fail` — 다른 스레드에서 불릴 수 있다.
pub(crate) fn show(title: &str, body: &str, on_fail: impl FnOnce() + Send + 'static) {
    #[cfg(target_os = "macos")]
    if mac::bundled() {
        let (t, b) = (title.to_string(), body.to_string());
        mac::post(title, body, move || osascript(&t, &b, on_fail));
        return;
    }
    osascript(title, body, on_fail);
}

/// AppleScript 문자열 리터럴 이스케이프 — 제목은 사용자(앞으로는 MCP·가져오기)가 쓴 글이라 스크립트 주입을 막는다.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn osascript(title: &str, body: &str, on_fail: impl FnOnce() + Send + 'static) {
    let script = format!("display notification \"{}\" with title \"{}\"", applescript_escape(body), applescript_escape(title));
    match std::process::Command::new("osascript").arg("-e").arg(script).spawn() {
        // 끝나길 기다리는 스레드 — 좀비 프로세스를 남기지 않고, 실패를 되돌린다.
        Ok(mut child) => {
            std::thread::spawn(move || match child.wait() {
                Ok(st) if st.success() => {}
                other => {
                    eprintln!("알림 실패: {other:?}");
                    on_fail();
                }
            });
        }
        Err(e) => {
            eprintln!("알림 실패: {e}");
            on_fail();
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{define_class, msg_send, AllocAnyThread};
    use objc2_foundation::{NSBundle, NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    /// 번들 id 가 있는 `.app` 안에서 도나. 맨 바이너리에서 UN 을 부르면 죽는다(머리 주석).
    pub(super) fn bundled() -> bool {
        let b = NSBundle::mainBundle();
        b.bundleIdentifier().is_some() && b.bundlePath().to_string().ends_with(".app")
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "OuroNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// 앱이 앞에 있어도(팝오버가 떠 있어도) 배너를 띄운다 — 안 그러면 macOS 가 조용히 알림 센터에만 넣는다.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _n: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
            }

            /// 알림을 누르면 팝오버를 연다.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _center: &UNUserNotificationCenter, _r: &UNNotificationResponse, handler: &block2::DynBlock<dyn Fn()>) {
                if let Some(f) = super::ON_CLICK.get() {
                    f();
                }
                handler.call(());
            }
        }
    );

    /// 델리게이트는 약한 참조라 누군가 붙들어야 한다 — 앱이 사는 동안 여기.
    static DELEGATE: Mutex<Option<Retained<Delegate>>> = Mutex::new(None);

    pub(super) fn setup() {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
        center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        *DELEGATE.lock().unwrap_or_else(|p| p.into_inner()) = Some(delegate);
        let done = RcBlock::new(|granted: Bool, err: *mut NSError| {
            if !granted.as_bool() {
                let why = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
                eprintln!("ouro: 알림 권한이 없어요 — osascript 로 띄웁니다 {why}");
            }
        });
        center.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &done);
    }

    // `Retained` 는 Send 가 아니지만 델리게이트는 만든 뒤 읽기만 한다 — static 에 두려면 필요하다.
    unsafe impl Send for Delegate {}

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// 알림 하나를 건다. 허용 안 됐거나 오류면 `fallback`(osascript). 완료 블록들은 UN 의 아무 스레드에서 한 번씩 불린다.
    pub(super) fn post(title: &str, body: &str, fallback: impl FnOnce() + Send + 'static) {
        let (title, body) = (title.to_string(), body.to_string());
        let fallback = Mutex::new(Some(fallback));
        let check = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            let take = || fallback.lock().unwrap_or_else(|p| p.into_inner()).take();
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            if status == UNAuthorizationStatus::Authorized || status == UNAuthorizationStatus::Provisional {
                if let Some(f) = take() {
                    add(&title, &body, f);
                }
            } else if let Some(f) = take() {
                f();
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&check);
    }

    fn add(title: &str, body: &str, fallback: impl FnOnce() + Send + 'static) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        let id = format!("ouro-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &content, None);
        // 블록은 Fn 이어야 하는데 fallback 은 FnOnce — 한 번만 꺼내 쓴다.
        let fallback = Mutex::new(Some(fallback));
        let done = RcBlock::new(move |err: *mut NSError| {
            if let Some(e) = unsafe { err.as_ref() } {
                eprintln!("ouro: 알림을 못 걸었어요 — {} (osascript 로)", e.localizedDescription());
                if let Some(f) = fallback.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    f();
                }
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applescript_escape_blocks_injection() {
        assert_eq!(applescript_escape(r#"a" & (do shell script "rm") & ""#), r#"a\" & (do shell script \"rm\") & \""#);
        assert_eq!(applescript_escape(r"back\slash"), r"back\\slash");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_binary_is_not_a_bundle() {
        // 테스트·`cargo run` 은 맨 바이너리 — UN 을 부르면 죽으니 osascript 길로 가야 한다.
        assert!(!mac::bundled());
    }
}
