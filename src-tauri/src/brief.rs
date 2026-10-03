// 브리핑 — «오늘 일정·겹침·빈 시간» 을 규칙으로 센다(PLAN §5 v0.2: 문장 요약에 AI 를 쓰지 않는다).
//
// 설계 결정:
//   · **전부 로컬 계산.** 일정이 바깥으로 나가지 않는다 — 비공개 일정도 센다(내 맥 안의 요약이라서).
//   · 빈 시간은 «낮 9시~18시 중 지금 이후, 30분 넘게 빈 틈» 앞의 둘. 종일 일정은 시간을 막지 않는다(`busy_between` 과 같은 규칙).
//   · 아침 알림은 하루 한 번, 8시~11시 사이 처음 깬 때. 오늘 아무것도 없으면 띄우지 않는다(빈 알림은 소음). 끌 수 있다(`settings`).

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use serde::Serialize;

use crate::alerts::clock;
use crate::store::{Event, Store};

const WORK_START: u32 = 9;
const WORK_END: u32 = 18;
const FREE_MIN_MS: i64 = 30 * 60 * 1000;
pub(crate) const MORNING_FROM: u32 = 8;
pub(crate) const MORNING_UNTIL: u32 = 11;
pub(crate) const KEY_ON: &str = "briefing";
pub(crate) const KEY_LAST: &str = "briefing_last";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Briefing {
    pub events: usize,
    pub errands: usize,
    /// 겹치는 일정 쌍의 수.
    pub conflicts: usize,
    /// 빈 시간 [시작, 끝) ms — 앞의 둘.
    pub free: Vec<(i64, i64)>,
    /// 짧은 구절들 — 팝오버는 « · » 로 잇고, 알림은 본문으로 쓴다.
    pub parts: Vec<String>,
    /// 아침 알림이 켜져 있나.
    pub morning: bool,
}

fn at(day: NaiveDate, h: u32) -> Option<i64> {
    Local.from_local_datetime(&day.and_time(NaiveTime::from_hms_opt(h, 0, 0)?)).earliest().map(|d| d.timestamp_millis())
}

/// 하루 요약 (순수 — 테스트 가능). `events` = 그날에 걸친 일정, `errands` = 그날 아직 안 돈 부탁 수.
pub(crate) fn summarize(events: &[Event], errands: usize, day: NaiveDate, now: i64) -> Briefing {
    let timed: Vec<(i64, i64, &str)> = events
        .iter()
        .filter(|e| !e.all_day)
        .filter_map(|e| Some((e.start_at?, e.end_at?, e.title.as_str())))
        .collect();
    let mut conflicts = 0;
    for (i, a) in timed.iter().enumerate() {
        for b in &timed[i + 1..] {
            if a.0 < b.1 && b.0 < a.1 {
                conflicts += 1;
            }
        }
    }
    let mut free = vec![];
    if let (Some(ws), Some(we)) = (at(day, WORK_START), at(day, WORK_END)) {
        let mut busy: Vec<(i64, i64)> = timed.iter().map(|t| (t.0, t.1.max(t.0))).collect();
        busy.sort();
        let mut cur = ws.max(now);
        for (s, e) in busy.into_iter().chain([(we, we)]) {
            let s = s.min(we);
            if s - cur >= FREE_MIN_MS && free.len() < 2 {
                free.push((cur, s));
            }
            cur = cur.max(e);
            if cur >= we {
                break;
            }
        }
    }
    let mut parts = vec![];
    if !events.is_empty() {
        parts.push(format!("일정 {}", events.len()));
    }
    if let Some(first) = timed.iter().filter(|t| t.0 >= now).min_by_key(|t| t.0) {
        parts.push(format!("다음 {} {}", clock(first.0), first.2));
    }
    if conflicts > 0 {
        parts.push(format!("겹침 {conflicts}"));
    }
    if errands > 0 {
        parts.push(format!("부탁 {errands}"));
    }
    if let Some((s, e)) = free.first() {
        parts.push(format!("빈 시간 {}–{}", clock(*s), clock(*e).replace("오전 ", "").replace("오후 ", "")));
    }
    Briefing { events: events.len(), errands, conflicts, free, parts, morning: true }
}

/// 오늘 브리핑 — DB 에서 읽어 센다.
pub(crate) fn today(store: &Store, now: i64) -> Result<Briefing, String> {
    let day = Local.timestamp_millis_opt(now).earliest().ok_or("시각이 이상해요")?.date_naive();
    let from = at(day, 0).ok_or("날짜가 이상해요")?;
    let to = at(day.succ_opt().ok_or("날짜가 이상해요")?, 0).ok_or("날짜가 이상해요")?;
    let events = store.list_events(from, to)?;
    let errands = store.list_errands(from, to)?.iter().filter(|e| e.run.is_none()).count();
    let mut b = summarize(&events, errands, day, now);
    b.morning = morning_on(store);
    Ok(b)
}

pub(crate) fn morning_on(store: &Store) -> bool {
    store.setting(KEY_ON).ok().flatten().as_deref() != Some("off")
}

/// 아침 알림을 띄울 때인가 — 켜져 있고, 8~11시이고, 오늘 아직 안 띄웠다. 띄우기로 했으면 «오늘 띄움» 을 먼저 적는다.
/// 돌려주는 값 = 알림 본문(오늘 아무것도 없으면 None — 적기만 하고 안 띄운다).
pub(crate) fn morning_due(store: &Store, now: i64) -> Option<String> {
    let local = Local.timestamp_millis_opt(now).earliest()?;
    let hour = chrono::Timelike::hour(&local);
    if !(MORNING_FROM..MORNING_UNTIL).contains(&hour) || !morning_on(store) {
        return None;
    }
    let key = local.date_naive().format("%Y-%m-%d").to_string();
    if store.setting(KEY_LAST).ok().flatten().as_deref() == Some(key.as_str()) {
        return None;
    }
    store.set_setting(KEY_LAST, &key).ok()?;
    let b = today(store, now).ok()?;
    (b.events + b.errands > 0).then(|| b.parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(title: &str, s: i64, e: i64) -> Event {
        Event {
            id: 1,
            title: title.into(),
            notes: String::new(),
            all_day: false,
            start_at: Some(s),
            end_at: Some(e),
            start_date: None,
            end_date: None,
            alert_min: None,
            private: false,
            origin: "user".into(),
            updated_at: 0,
        }
    }

    #[test]
    fn counts_conflicts_and_free_gaps() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let h = |x: u32| at(day, x).unwrap();
        let events = vec![ev("회의", h(10), h(11)), ev("점심", h(10) + 30 * 60_000, h(12)), ev("치과", h(15), h(16))];
        let b = summarize(&events, 2, day, h(8));
        assert_eq!((b.events, b.conflicts, b.errands), (3, 1, 2));
        assert_eq!(b.free, vec![(h(9), h(10)), (h(12), h(15))], "9~10, 12~15 (16~18 은 셋째라 안 센다)");
        assert!(b.parts.iter().any(|p| p == "다음 오전 10:00 회의"), "{:?}", b.parts);
        // 지금이 13시면 빈 시간은 13시부터.
        let b = summarize(&events, 0, day, h(13));
        assert_eq!(b.free, vec![(h(13), h(15)), (h(16), h(18))]);
        assert!(!b.parts.iter().any(|p| p.starts_with("부탁")));
    }

    #[test]
    fn empty_day_is_quiet() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let b = summarize(&[], 0, day, at(day, 20).unwrap());
        assert!(b.parts.is_empty() && b.free.is_empty());
    }

    #[test]
    fn morning_once_per_day_and_can_be_off() {
        let s = Store::open_in_memory();
        let day = Local::now().date_naive();
        let nine = at(day, 9).unwrap();
        // 아무 일정도 없으면 «띄움» 만 적고 본문은 없다.
        assert_eq!(morning_due(&s, nine), None);
        assert_eq!(s.setting(KEY_LAST).unwrap(), Some(day.format("%Y-%m-%d").to_string()));
        s.set_setting(KEY_LAST, "2000-01-01").unwrap();
        assert_eq!(morning_due(&s, at(day, 7).unwrap()), None, "8시 전엔 안 띄운다");
        s.set_setting(KEY_ON, "off").unwrap();
        assert_eq!(morning_due(&s, nine), None);
        assert_eq!(s.setting(KEY_LAST).unwrap().as_deref(), Some("2000-01-01"), "꺼져 있으면 적지도 않는다");
    }
}
