// 알림 + 매일 백업 — 앱 안에서 도는 작은 시계 하나.
//
// 설계 결정:
//   · 상태는 DB 에만 있다(`alerts_sent`). 스레드는 «지금 울릴 게 있나» 만 본다 — 앱을 껐다 켜도,
//     일정을 고쳐도 같은 규칙으로 판단한다. 일정 시각을 옮기면 울릴 시각이 달라져 새 알림으로 다시 걸린다.
//   · **늦은 알림은 10분까지만.** 맥이 자는 동안 지난 알림을 깨자마자 몇 시간 치 쏟아 내는 건 소음이다.
//     (부탁은 다르다 — 놓친 부탁은 «늦게 실행 / 건너뜀» 을 부탁마다 고른다, PLAN §9-3. 개발 5.)
//   · 잠은 **최대 30초씩** 끊어 잔다. macOS 에서 스레드 잠(recv_timeout)은 단조 시계라 **맥이 자는 동안 멈춘다** —
//     「다음 알림까지 2시간 자기」 는 맥이 1시간 잤다면 1시간 늦게 깬다. 짧게 끊으면 깨어난 뒤 30초 안에 따라잡는다.
//   · 일정이 바뀌면 `Alerts::poke` 로 바로 깨운다 — 3분 뒤 일정을 만들며 «1분 전» 을 걸었을 때 다음 30초를 기다리지 않게.
//   · 알림은 osascript 로 띄운다. tauri-plugin-notification(notify-rust)은 옛 NSUserNotificationCenter 를 써서
//     macOS 26 이 조용히 버린다 — show() 가 Ok 인데 화면엔 안 뜬다(Kura notify.rs 실측). 정석은 서명 번들 + UNUserNotificationCenter(개발 8).

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Local, NaiveDate, TimeZone, Timelike, Utc};

use crate::store::{self, Event, Store, ALL_DAY_ALERT_HOUR};

/// 울릴 시각을 이만큼 넘겨도 띄운다. 이보다 늦으면 건너뛴다.
const LATE_LIMIT_MS: i64 = 10 * 60 * 1000;
/// 한 번에 자는 최대 시간(위 머리 주석 — 잠든 맥).
const MAX_NAP: Duration = Duration::from_secs(30);
/// 백업 보관 개수(하루 하나). PLAN §6.
const BACKUP_KEEP: usize = 7;

/// 스케줄러 손잡이 — 일정이 바뀌면 `poke`.
pub(crate) struct Alerts {
    tx: Sender<()>,
    /// 마지막 백업 실패 문장. 성공하면 비운다. 팝오버가 읽어 한 줄로 보인다 —
    /// DB 가 유일한 원본이라(PLAN §6) 백업이 며칠 조용히 멈춰 있으면 안 된다(코덱스 개발 2).
    backup_error: Arc<Mutex<Option<String>>>,
}

impl Alerts {
    pub(crate) fn poke(&self) {
        let _ = self.tx.send(());
    }

    pub(crate) fn backup_error(&self) -> Option<String> {
        self.backup_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// 일정이 울릴 시각(UTC ms). 알림이 없으면 None.
/// 시각 일정 = 시작 − N분. 종일 일정 = 시작일 로컬 오전 9시 − N분(0 = 당일 9시, 1440 = 전날 9시).
pub(crate) fn fire_at(e: &Event) -> Option<i64> {
    let m = e.alert_min?;
    let base = if e.all_day {
        let d = NaiveDate::parse_from_str(e.start_date.as_deref()?, "%Y-%m-%d").ok()?;
        let t = d.and_hms_opt(ALL_DAY_ALERT_HOUR, 0, 0)?;
        // 서머타임으로 9시가 두 번이면 앞의 것, 없으면(건너뛴 시간) None — 9시엔 전환이 없으니 사실상 항상 하나다.
        Local.from_local_datetime(&t).earliest()?.timestamp_millis()
    } else {
        e.start_at?
    };
    Some(base - m * 60_000)
}

/// 지금 띄울 것 (순수 — 테스트 가능): 울릴 시각이 (now − 10분, now] 안인 것.
fn due(events: &[Event], now: i64) -> Vec<(i64, &Event)> {
    events
        .iter()
        .filter_map(|e| fire_at(e).map(|f| (f, e)))
        .filter(|&(f, _)| f <= now && now - f <= LATE_LIMIT_MS)
        .collect()
}

/// 다음 알림까지 남은 시간 — 없거나 멀면 MAX_NAP.
fn nap(events: &[Event], now: i64) -> Duration {
    events
        .iter()
        .filter_map(fire_at)
        .filter(|&f| f > now)
        .min()
        .map(|f| Duration::from_millis((f - now) as u64).min(MAX_NAP))
        .unwrap_or(MAX_NAP)
}

/// 알림 문구 (순수 — 테스트 가능). 제목 = 일정 제목, 본문 = 언제.
fn notice(e: &Event, now: i64) -> (String, String) {
    let body = if e.all_day {
        match e.alert_min {
            Some(1440) => "내일 · 종일".to_string(),
            _ => "오늘 · 종일".to_string(),
        }
    } else {
        let start = e.start_at.unwrap_or(now);
        let at = Local
            .timestamp_millis_opt(start)
            .earliest()
            .map(|d| {
                let (ampm, h12) = match d.hour() {
                    0 => ("오전", 12),
                    h @ 1..=11 => ("오전", h),
                    12 => ("오후", 12),
                    h => ("오후", h - 12),
                };
                format!("{ampm} {h12}:{}", d.format("%M"))
            })
            .unwrap_or_default();
        let mins = (start - now + 30_000).div_euclid(60_000);
        if mins <= 0 {
            format!("지금 · {at}")
        } else if mins < 60 {
            format!("{mins}분 후 · {at}")
        } else if mins % 60 == 0 && mins < 24 * 60 {
            format!("{}시간 후 · {at}", mins / 60)
        } else {
            at
        }
    };
    (e.title.clone(), body)
}

/// 알림을 띄운다. 실패하면(osascript 를 못 띄우거나 오류로 끝나면) `on_fail` — 끝나길 기다리는 스레드에서 부른다.
fn show_notification(title: &str, body: &str, on_fail: impl FnOnce() + Send + 'static) {
    // AppleScript 문자열 리터럴 이스케이프 — 제목은 사용자(앞으로는 MCP·가져오기)가 쓴 글이라 스크립트 주입을 막는다.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
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

/// 백업 파일 이름: `ouro-YYYY-MM-DD.db`. 이름순 = 날짜순이라 정리가 쉽다.
fn backup_name(d: NaiveDate) -> String {
    format!("ouro-{}.db", d.format("%Y-%m-%d"))
}

/// 오늘 백업이 없으면 만들고, 7개 넘는 옛것을 지운다. 같은 날 두 번 부르면 아무것도 안 한다.
pub(crate) fn backup_daily(store: &Store, dir: &std::path::Path, today: NaiveDate) -> Result<(), String> {
    store::create_private_dir(dir)?;
    let dest = dir.join(backup_name(today));
    if dest.exists() {
        return Ok(());
    }
    // 임시 이름으로 만든 뒤 옮긴다 — 도중에 꺼져도 반쪽 파일이 «오늘 백업» 행세를 하지 않게.
    let tmp = dir.join(format!(".{}.tmp", backup_name(today)));
    let _ = std::fs::remove_file(&tmp); // VACUUM INTO 는 이미 있는 파일에 못 쓴다
    store.snapshot_to(&tmp)?;
    store::restrict_file(&tmp)?;
    std::fs::rename(&tmp, &dest).map_err(|e| format!("백업 이름 바꾸기 실패: {e}"))?;
    prune_backups(dir);
    Ok(())
}

fn prune_backups(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with("ouro-") && n.ends_with(".db") && n.len() == "ouro-YYYY-MM-DD.db".len())
        .collect();
    names.sort();
    let extra = names.len().saturating_sub(BACKUP_KEEP);
    for n in &names[..extra] {
        let _ = std::fs::remove_file(dir.join(n));
    }
}

/// 시계 스레드를 띄운다.
pub(crate) fn start(store: Arc<Store>, backup_dir: PathBuf) -> Alerts {
    let (tx, rx) = mpsc::channel::<()>();
    let backup_error = Arc::new(Mutex::new(None));
    let backup_err = backup_error.clone();
    std::thread::Builder::new()
        .name("ouro-alerts".into())
        .spawn(move || {
            let mut backed_up: Option<NaiveDate> = None;
            loop {
                let now = Utc::now().timestamp_millis();

                let today = Local::now().date_naive();
                if backed_up != Some(today) {
                    let res = backup_daily(&store, &backup_dir, today);
                    if res.is_ok() {
                        backed_up = Some(today);
                    }
                    // 실패하면 다음 바퀴에 다시 — 디스크가 잠깐 찼을 수 있다.
                    *backup_err.lock().unwrap_or_else(|p| p.into_inner()) = res.err().map(|e| format!("오늘 백업을 못 했어요 — {e}"));
                }

                let events = store
                    .alert_candidates(now - LATE_LIMIT_MS, now + MAX_NAP.as_millis() as i64)
                    .unwrap_or_else(|e| {
                        eprintln!("알림 후보 읽기 실패: {e}");
                        Vec::new()
                    });
                for (f, e) in due(&events, now) {
                    // 먼저 적고 띄운다 — 적기에 실패하면 띄우지 않는다(같은 알림이 30초마다 반복되는 것보다 낫다).
                    if let Ok(true) = store.mark_alert_sent(e.id, f) {
                        let (title, body) = notice(e, now);
                        let (st, id) = (store.clone(), e.id);
                        show_notification(&title, &body, move || st.unmark_alert_sent(id, f));
                    }
                }

                match rx.recv_timeout(nap(&events, now)) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {
                        // 몰려온 poke 는 한 번으로 친다.
                        while rx.try_recv().is_ok() {}
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .expect("알림 스레드를 띄우지 못했어요");
    Alerts { tx, backup_error }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(start_at: Option<i64>, start_date: Option<&str>, alert: Option<i64>) -> Event {
        Event {
            id: 1,
            title: "치과".into(),
            notes: String::new(),
            all_day: start_date.is_some(),
            start_at,
            end_at: start_at,
            start_date: start_date.map(Into::into),
            end_date: start_date.map(|_| "2099-01-01".into()),
            alert_min: alert,
            private: false,
            origin: "user".into(),
            updated_at: 0,
        }
    }

    const MIN: i64 = 60_000;

    #[test]
    fn fire_at_timed_and_all_day() {
        assert_eq!(fire_at(&ev(Some(100 * MIN), None, Some(10))), Some(90 * MIN));
        assert_eq!(fire_at(&ev(Some(100 * MIN), None, None)), None);
        let nine = Local.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).earliest().unwrap().timestamp_millis();
        assert_eq!(fire_at(&ev(None, Some("2026-09-28"), Some(0))), Some(nine));
        assert_eq!(fire_at(&ev(None, Some("2026-09-28"), Some(1440))), Some(nine - 1440 * MIN));
    }

    #[test]
    fn due_window_is_ten_minutes_late_at_most() {
        let e = [ev(Some(100 * MIN), None, Some(0))];
        assert!(due(&e, 100 * MIN - 1).is_empty(), "아직");
        assert_eq!(due(&e, 100 * MIN).len(), 1, "정각");
        assert_eq!(due(&e, 110 * MIN).len(), 1, "10분 늦음까지");
        assert!(due(&e, 110 * MIN + 1).is_empty(), "그보다 늦으면 건너뜀");
    }

    #[test]
    fn nap_is_capped_and_aims_at_next_alert() {
        let e = [ev(Some(100 * MIN), None, Some(0))];
        assert_eq!(nap(&e, 100 * MIN - 5_000), Duration::from_secs(5));
        assert_eq!(nap(&e, 0), MAX_NAP);
        assert_eq!(nap(&[], 0), MAX_NAP);
    }

    #[test]
    fn notice_says_when() {
        let start = Local.with_ymd_and_hms(2026, 9, 28, 15, 5, 0).earliest().unwrap().timestamp_millis();
        let e = ev(Some(start), None, Some(10));
        assert_eq!(notice(&e, start - 10 * MIN), ("치과".into(), "10분 후 · 오후 3:05".into()));
        assert_eq!(notice(&e, start).1, "지금 · 오후 3:05");
        assert_eq!(notice(&e, start - 120 * MIN).1, "2시간 후 · 오후 3:05");
        assert_eq!(notice(&e, start - 1440 * MIN).1, "오후 3:05");
        let morning = Local.with_ymd_and_hms(2026, 9, 28, 0, 30, 0).earliest().unwrap().timestamp_millis();
        assert_eq!(notice(&ev(Some(morning), None, Some(0)), morning).1, "지금 · 오전 12:30");
        assert_eq!(notice(&ev(None, Some("2026-09-28"), Some(1440)), 0).1, "내일 · 종일");
    }

    #[test]
    fn backup_once_a_day_keeps_seven() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("data")).unwrap();
        let bdir = dir.path().join("backup");
        for d in 1..=9 {
            backup_daily(&s, &bdir, NaiveDate::from_ymd_opt(2026, 9, d).unwrap()).unwrap();
        }
        // 같은 날 두 번은 아무 일도 없다.
        backup_daily(&s, &bdir, NaiveDate::from_ymd_opt(2026, 9, 9).unwrap()).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&bdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names.len(), 7);
        assert_eq!(names[0], "ouro-2026-09-03.db");
        assert_eq!(names[6], "ouro-2026-09-09.db");
    }
}
