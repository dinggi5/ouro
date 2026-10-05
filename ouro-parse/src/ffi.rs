// C 로 여는 문 — iOS 가 스위프트에서 부른다(`ouro-parse/swift`, 개발 14). 글 JSON 하나를 주고받는다:
// 칸을 C 구조체로 풀면 칸이 늘 때마다 헤더·스위프트·러스트 세 곳을 같이 고쳐야 하는데, JSON 이면 `Draft` 의 serde 하나다.
//
// 약속:
//   · 넘겨받는 글은 전부 NUL 로 끝나는 UTF-8. `now` = 로컬 지금 `YYYY-MM-DDTHH:MM:SS`, `base` = `YYYY-MM-DD` 또는 NULL.
//   · 돌려주는 포인터는 `ouro_free` 로만 푼다(러스트 할당자). 입력이 틀렸으면 NULL — 패닉은 바깥으로 넘기지 않는다.

use std::ffi::{c_char, CStr, CString};
use std::panic::catch_unwind;

use chrono::{NaiveDate, NaiveDateTime};

unsafe fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: 부르는 쪽이 NUL 로 끝나는 글을 준다(위 약속).
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// 「내일 3시 치과」 → `Draft` JSON. 입력이 틀렸으면 NULL.
///
/// # Safety
/// `text`·`now` 는 NUL 로 끝나는 글, `base` 는 그런 글이거나 NULL 이어야 한다.
#[no_mangle]
pub unsafe extern "C" fn ouro_parse(text: *const c_char, now: *const c_char, base: *const c_char) -> *mut c_char {
    // SAFETY: 위 # Safety.
    let (Some(t), Some(n)) = (unsafe { self::text(text) }, unsafe { self::text(now) }) else {
        return std::ptr::null_mut();
    };
    let b = if base.is_null() {
        None
    } else {
        // base 를 줬는데 못 읽으면 조용히 «오늘 기준» 으로 바꾸지 않는다 — 다른 날로 들어가는 게 최악이다.
        match unsafe { self::text(base) }.and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()) {
            Some(d) => Some(d),
            None => return std::ptr::null_mut(),
        }
    };
    let Ok(now) = NaiveDateTime::parse_from_str(n, "%Y-%m-%dT%H:%M:%S") else {
        return std::ptr::null_mut();
    };
    let out = catch_unwind(|| serde_json::to_string(&crate::parse(t, now, b)).ok());
    match out {
        Ok(Some(json)) => CString::new(json).map(CString::into_raw).unwrap_or(std::ptr::null_mut()),
        _ => std::ptr::null_mut(),
    }
}

/// `ouro_parse` 가 돌려준 글을 푼다. NULL 이면 아무것도 안 한다.
///
/// # Safety
/// `ouro_parse` 가 돌려준 포인터(또는 NULL)여야 하고, 한 번만 푼다.
#[no_mangle]
pub unsafe extern "C" fn ouro_free(p: *mut c_char) {
    if !p.is_null() {
        // SAFETY: 위 # Safety — `CString::into_raw` 로 내준 것.
        drop(unsafe { CString::from_raw(p) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(text: &str, now: &str, base: Option<&str>) -> Option<String> {
        let t = CString::new(text).unwrap();
        let n = CString::new(now).unwrap();
        let b = base.map(|b| CString::new(b).unwrap());
        unsafe {
            let p = ouro_parse(t.as_ptr(), n.as_ptr(), b.as_ref().map_or(std::ptr::null(), |b| b.as_ptr()));
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_str().unwrap().to_string();
            ouro_free(p);
            Some(s)
        }
    }

    #[test]
    fn round_trip_json() {
        let j = call("내일 3시 치과", "2026-09-29T10:00:00", None).unwrap();
        assert_eq!(
            j,
            r#"{"title":"치과","allDay":false,"startDate":"2026-09-30","startTime":"15:00","endDate":"2026-09-30","endTime":"16:00","warnings":[],"miss":null}"#
        );
        // 보고 있는 날이 있으면 날짜 없는 시각이 그날로.
        let j = call("3시 치과", "2026-09-29T10:00:00", Some("2026-10-05")).unwrap();
        assert!(j.contains(r#""startDate":"2026-10-05""#), "{j}");
    }

    #[test]
    fn bad_input_is_null() {
        assert!(call("치과", "어제", None).is_none());
        assert!(call("치과", "2026-09-29T10:00:00", Some("10/05")).is_none());
        unsafe {
            assert!(ouro_parse(std::ptr::null(), std::ptr::null(), std::ptr::null()).is_null());
            ouro_free(std::ptr::null_mut());
        }
    }
}
