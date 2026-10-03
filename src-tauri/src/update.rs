// 인앱 업데이트 (개발 8, Kura `src-tauri/src/update.rs` 에서 가져옴) — 확인·내려받기·설치.
//
// 업데이트 서명 개인키를 쥔 쪽은 이 앱으로 임의 코드를 밀어 넣을 수 있고, 그 코드는 `~/.ouro/ouro.db`(일정·부탁·답)를 읽고
// `claude`·`codex` 를 돌릴 수 있다. 그래서 편의보다 통제가 먼저다:
//
//   1. **조용한 설치를 안 한다.** 확인과 설치가 다른 커맨드고, 설치는 사람이 버전과 릴리스 노트를 본 뒤 누를 때만 돈다.
//      저절로 도는 건 «확인» 까지(팝오버가 켤 때·하루 한 번 부른다).
//   2. **웹뷰에 updater 권한을 안 준다.** capabilities 에 `updater:default` 가 없다 — 넣으면 프론트가 플러그인을 직접 불러
//      아래 가드를 건너뛴다. 러스트의 `app.updater()` 는 ACL 을 안 거친다. 프론트가 쓸 수 있는 건 이 파일의 커맨드뿐.
//   3. **서명 검증은 못 끈다.** 플러그인이 minisign 으로 강제한다(tauri.conf.json 의 pubkey).
//   4. **부탁이 도는 중엔 설치를 막는다.** 설치 끝에 앱이 다시 켜지는데, 그때 돌던 `claude`·`codex` 는 답을 못 적고
//      기록은 «끊김» 으로 닫힌다(개발 6 의 «강제 종료 뒤 자식 생존» 문제도 그대로 밟는다). 받는 사이에 시작된 것도 다시 본다.
//      일정·부탁 제안(MCP)은 DB 에 있어 재시작해도 남는다 — 기다리던 AI 쪽 연결만 끊긴다.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 진행률 이벤트. payload = `UpdateProgress`.
const PROGRESS_EVENT: &str = "update-progress";

/// `check_update` 가 찾은 업데이트를 `install_update` 까지 들고 있는 자리.
/// 설치 때 다시 `check()` 하면 그 사이 릴리스가 바뀌었을 때 **사람이 본 버전과 다른 것**을 깔게 된다 — 보여 준 그 객체로만 깐다.
#[derive(Default)]
pub(crate) struct PendingUpdate(pub(crate) Mutex<Option<Update>>);

/// 사람이 설치를 누르기 전에 보는 값 — 여기 있는 것만이 판단 근거다.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateInfo {
    pub(crate) version: String,
    pub(crate) current_version: String,
    pub(crate) notes: Option<String>,
}

#[derive(Serialize, Clone)]
struct UpdateProgress {
    downloaded: u64,
    /// 서버가 Content-Length 를 안 주면 없다.
    total: Option<u64>,
}

/// 새 버전이 있나. 있으면 정보를 돌려주고 객체를 담아 둔다. 네트워크가 없으면 Err — 자주 있는 일이라 프론트는 저절로 확인할 땐 조용히 넘긴다.
#[tauri::command]
pub(crate) async fn check_update(app: AppHandle, state: State<'_, PendingUpdate>) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| format!("업데이트 확인을 준비하지 못했어요: {e}"))?;
    let found = updater.check().await.map_err(|e| format!("업데이트를 확인하지 못했어요: {e}"))?;
    // 잠금은 await 뒤에만 잡는다(std Mutex 가드를 await 너머로 들고 가지 않는다).
    let mut slot = state.0.lock().map_err(|_| "업데이트 상태가 깨졌어요".to_string())?;
    Ok(match found {
        Some(update) => {
            let info = UpdateInfo {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
                notes: update.body.clone(),
            };
            *slot = Some(update);
            Some(info)
        }
        None => {
            // 최신이면 들고 있던 옛 후보를 버린다 — 릴리스가 내려간 뒤에도 설치 버튼이 살아 있지 않게.
            *slot = None;
            None
        }
    })
}

/// 담아 둔 업데이트를 받아 깔고 앱을 다시 켠다.
#[tauri::command]
pub(crate) async fn install_update(
    app: AppHandle,
    state: State<'_, PendingUpdate>,
    core: State<'_, crate::CoreState>,
) -> Result<(), String> {
    let busy = || -> Result<bool, String> {
        match core.inner() {
            Ok(c) => c.store.any_running(),
            // DB 를 못 연 앱은 부탁을 돌릴 수도 없다 — 업데이트가 그 문제를 고칠 수도 있으니 막지 않는다.
            Err(_) => Ok(false),
        }
    };
    if busy()? {
        return Err("부탁이 도는 중이에요. 끝난 뒤에 업데이트하세요.".into());
    }
    let update = state
        .0
        .lock()
        .map_err(|_| "업데이트 상태가 깨졌어요".to_string())?
        .take()
        .ok_or_else(|| "설치할 업데이트가 없어요. 다시 확인해 주세요.".to_string())?;

    // 받기와 깔기를 나눈다 — 한 번에 하면 위 검사와 재시작 사이에 내려받기(수십 초)가 통째로 끼어,
    // 그 사이 시작된 부탁이 재시작에 휩쓸린다(Kura 코덱스 P1 과 같은 틈).
    let progress_app = app.clone();
    let mut downloaded: u64 = 0;
    let bytes = match update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit(PROGRESS_EVENT, UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
    {
        Ok(b) => b,
        Err(e) => return Err(restore_slot(&state, update, format!("업데이트를 내려받지 못했어요: {e}"))),
    };
    // 여기가 되돌릴 수 있는 마지막 자리다 — 아직 앱을 안 건드렸고 받은 바이트는 메모리에만 있다.
    match busy() {
        Ok(false) => {}
        Ok(true) => {
            return Err(restore_slot(&state, update, "받는 사이에 부탁이 돌기 시작했어요. 끝난 뒤에 다시 누르세요.".into()))
        }
        Err(e) => return Err(restore_slot(&state, update, e)),
    }
    if let Err(e) = update.install(bytes) {
        return Err(restore_slot(&state, update, format!("업데이트를 설치하지 못했어요: {e}")));
    }
    // 새 번들이 깔렸다. 다시 켜야 새 코드가 돈다. restart() 는 돌아오지 않는다.
    app.restart()
}

/// 실패한 업데이트를 자리에 되돌리고 오류 문장을 돌려준다 — 사람이 본 그 버전으로 다시 누를 수 있게.
fn restore_slot(state: &State<'_, PendingUpdate>, update: Update, msg: String) -> String {
    if let Ok(mut slot) = state.0.lock() {
        *slot = Some(update);
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_update_starts_empty() {
        assert!(PendingUpdate::default().0.lock().unwrap().is_none(), "확인 전에 설치가 되면 안 된다");
    }
}
