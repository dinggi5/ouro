// 빠른 입력 규칙 파서 — 「내일 3시 치과」 → 일정 초안 (PLAN §7, 개발 3).
// 개발 14 에 `src-tauri/src/parse.rs` 에서 크레이트로 떼었다 — 맥 앱과 iOS(`ffi`, xcframework)가 이 파일 하나를 쓴다.
//
// 설계 결정:
//   · **날짜 계산은 여기서만, 결정적으로.** 같은 글 + 같은 «지금» = 언제나 같은 답. 나중에 모델을 붙여도 모델은 표현만 뽑고
//     계산은 이 파일이 한다(`Engine` 이 그 자리).
//   · **추측하지 않는다.** 날짜를 못 찾으면 칸을 비운 초안을 돌려주고, 프론트가 시트를 열어 사람이 고른다 —
//     틀린 날짜로 조용히 저장되는 게 최악이다. 글 중 날짜로 **쓰지 않은** 조각은 제목에 그대로 남긴다(잃지 않게).
//   · 추측하는 곳은 넷이고 전부 카드에 날짜·오전/오후로 드러난다:
//     ① 오전/오후 없는 1~6시 = 오후(«3시 치과»), ② 날짜 없는 시각이 오늘 이미 지났으면 내일,
//     ③ 연도 없는 날짜가 지났으면 내년, 달 없는 «15일» 이 지났으면 다음 달, ④ 요일만 있으면 오늘부터 가장 가까운 그 요일.
//   · «이번 주·다음 주» 는 **월요일 시작**으로 센다 — 말로 하는 «이번 주 일요일» 은 다가오는 일요일이다.
//     달력 칸(일요일 시작, `time.ts` startOfWeek)과 다르지만, 칸은 한국 달력 관례고 이건 한국어 말 관례다.
//   · 반복(매주·매일)은 알아듣되 아직 못 만든다(RRULE 은 v0.2) — 첫 번 한 번만 넣고 카드에 그렇게 적는다.
//   · **경고(`warnings`)가 하나라도 있으면 프론트는 바로 넣지 않고 시트를 연다**(코덱스 개발 3) — 없는 날짜를 빼고 남은
//     시각, 거꾸로 쓴 범위의 시작만, 반복의 첫 번 — 전부 «사람이 쓴 것과 다른 일정» 이라 사람이 보고 저장해야 한다.
//
// 흐름: 글에서 조각(날짜·시각·길이·«N분 뒤»)을 뽑고 → 엮고(범위 «~», 오전/오후 물려받기) → 초안.
// 조각을 뽑은 자리는 같은 바이트 수의 공백으로 가려 다음 규칙이 같은 글자를 두 번 먹지 않게 한다(위치가 안 어긋난다).

#[cfg(feature = "ffi")]
pub mod ffi;

use std::ops::Range;
use std::sync::LazyLock;

use chrono::{Datelike, Days, Duration, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

/// 파서가 돌려주는 초안 — 프론트 `EventSheet` 의 Draft 와 같은 «화면 값» 모양(로컬 날짜·시각 문자열).
/// UTC 로 바꾸는 건 프론트의 `combine` 이 한다(서머타임에 없는 시각을 거기서 거른다).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub title: String,
    pub all_day: bool,
    /// 날짜를 못 찾았으면 넷 다 None — 사람이 시트에서 고른다.
    pub start_date: Option<String>,
    pub start_time: Option<String>,
    /// 종일이면 **포함**(9/28 ~ 9/28 = 하루). 저장할 때 배타로 바꾸는 건 프론트 `toInput`.
    pub end_date: Option<String>,
    pub end_time: Option<String>,
    /// 카드에 보일 한 줄들(반복 못 함, 없는 날짜 …).
    pub warnings: Vec<String>,
    /// 못 알아들은 유형 — 입력을 확정할 때 개수만 센다(PLAN §7, 본문은 저장하지 않는다).
    pub miss: Option<Miss>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Miss {
    /// 날짜·시각 표현을 하나도 못 찾음.
    NoDate,
    /// 찾았는데 없는 값(2월 30일, 25시).
    Invalid,
    /// 반복 표현 — 아직 못 만든다.
    Repeat,
}

impl Miss {
    pub fn key(self) -> &'static str {
        match self {
            Miss::NoDate => "no_date",
            Miss::Invalid => "invalid",
            Miss::Repeat => "repeat",
        }
    }
}

/// 엔진 자리(PLAN §7). MVP 는 규칙 하나뿐이다 — 나중에 로컬 모델을 붙이면 표현을 뽑는 앞단으로 여기 들어온다.
pub trait Engine {
    /// `now` = 로컬 지금. `base` = 사람이 보고 있는 날(오늘이 아니면) — 날짜 없는 «3시 치과» 가 그날로 간다.
    fn parse(&self, text: &str, now: NaiveDateTime, base: Option<NaiveDate>) -> Draft;
}

pub struct Rules;

impl Engine for Rules {
    fn parse(&self, text: &str, now: NaiveDateTime, base: Option<NaiveDate>) -> Draft {
        parse(text, now, base)
    }
}

// ───────────────────────── 조각 ─────────────────────────

/// 조각 끝에 붙은 조사 — 범위를 엮을 때 쓴다(«3시부터 5시까지»).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Link {
    None,
    From,
    To,
}

#[derive(Debug, Clone)]
struct Tok<T> {
    v: T,
    span: Range<usize>,
    link: Link,
}

#[derive(Debug, Clone, Copy)]
enum DateTok {
    /// 오늘 = 0, 내일 = 1, 3일 뒤 = 3, 2주 뒤 = 14.
    Days(i64),
    /// 한 달 뒤 = 1.
    Months(u32),
    /// 연·월이 없으면 가장 가까운 앞날로 굴린다.
    Ymd { y: Option<i32>, m: Option<u32>, d: u32 },
    /// 다음 달 3일 = { months: 1, d: 3 }.
    MonthDay { months: i32, d: u32 },
    /// weeks = None → 오늘부터 가장 가까운 그 요일. Some(k) → 이번 주(월요일 시작)에서 k주.
    Weekday { weeks: Option<i64>, wd: Weekday },
    /// 토~일. 시각이 붙으면 토요일 하루.
    Weekend { weeks: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Period {
    Am,
    Pm,
    /// 낮 — 1~6 은 오후, 7~12 는 그대로.
    Day,
    /// 밤 — 6~11 은 오후, 12·1~5 는 다음 날 새벽.
    Night,
}

#[derive(Debug, Clone, Copy)]
struct TimeTok {
    period: Option<Period>,
    h: u32,
    m: u32,
    /// 24시간제로 적었다(앞자리 0 «08:30», 13시 이상) — 오후로 굴리지 않는다.
    fixed: bool,
}

struct Scan {
    masked: String,
    dates: Vec<Tok<DateTok>>,
    times: Vec<Tok<TimeTok>>,
    /// 길이(분). 시작 시각이 있고 끝이 없을 때만 쓴다.
    durs: Vec<Tok<i64>>,
    /// «30분 뒤» — 지금부터(분).
    rels: Vec<Tok<i64>>,
    repeat: Vec<Range<usize>>,
}

// ───────────────────────── 규칙 ─────────────────────────

/// 조각 끝 조사. «마다» 는 반복 표시.
const PARTICLE: &str = r"(?P<link>에는|에|은|는|부터|까지|쯤에?|경에?|께|마다)?";
const PERIOD: &str = r"(?:(?P<p>오전|오후|아침|낮|저녁|밤|새벽)\s*)?";
/// 시(時) 앞에 오는 우리말 수. 긴 것을 앞에(정규식은 왼쪽 먼저 고른다).
const NATIVE_HOUR: &str = "열한|열두|열|한|두|세|네|다섯|여섯|일곱|여덟|아홉";

fn re(core: &str) -> Regex {
    Regex::new(&format!("{core}{PARTICLE}")).expect("parse 규칙 정규식")
}

static REL_TIME: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?P<n>[0-9]{1,3}|열|한|두|세|네|다섯|여섯)\s*(?P<u>시간|분)(?P<half>\s*반)?\s*(?:뒤|후|있다가)")
});
static REL_DAYS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?P<n>[0-9]{1,3}|일|한|두|세|네|다섯|여섯|일곱|여덟|아홉|열)\s*(?P<u>주일|일|주|달|개월)\s*(?:뒤|후|있다가|지나서)")
});
static NATIVE_DAYS: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?P<w>하루|이틀|사흘|나흘|닷새|엿새|이레|열흘)\s*(?:뒤|후|있다가|지나서)"));
static DATE_WORD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?P<w>내일모레|낼모레|그저께|그제|어제|오늘|내일|낼|모레|글피)"));
static YMD_SEP: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?P<y>[0-9]{4})[-./](?P<m>[0-9]{1,2})[-./](?P<d>[0-9]{1,2})\.?"));
static YMD_KO: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:(?P<y>[0-9]{4})\s*년\s*)?(?P<m>[0-9]{1,2})\s*월\s*(?P<d>[0-9]{1,2})\s*일")
});
static MD_SLASH: LazyLock<Regex> = LazyLock::new(|| re(r"(?P<m>[0-9]{1,2})/(?P<d>[0-9]{1,2})"));
static MD_DOT: LazyLock<Regex> = LazyLock::new(|| re(r"(?P<m>[0-9]{1,2})\.(?P<d>[0-9]{1,2})\.?"));
static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?P<q>이번|이|다음|담|다다음|지난|저번)\s*달\s*(?P<d>[0-9]{1,2})\s*일")
});
static WEEKEND: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:(?P<q>이번|다음|담|다다음|지난|저번)\s*)?주말"));
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?:(?P<q>이번|다음|담|다다음|지난|저번|매)\s*(?P<wk>주)?\s*)?(?P<wd>[월화수목금토일])(?P<yo>요일)?")
});
static BARE_DAY: LazyLock<Regex> = LazyLock::new(|| re(r"(?P<d>[0-9]{1,2})\s*일"));
static REPEAT: LazyLock<Regex> = LazyLock::new(|| re(r"매\s*(?:일|주|달|월|년)|격주|평일"));
static RANGE_SHORT: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(r"{PERIOD}(?P<h1>[0-9]{{1,2}})\s*[-~〜]\s*(?P<h2>[0-9]{{1,2}})\s*시"))
});
static CLOCK: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"{PERIOD}(?P<h>[0-9]{{1,2}}):(?P<m>[0-9]{{2}})")));
static HOUR: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"{PERIOD}(?P<h>[0-9]{{1,2}}|{NATIVE_HOUR})\s*시(?:\s*(?P<half>반)|\s*(?P<m>[0-5]?[0-9])\s*분)?"
    ))
});
static NOON: LazyLock<Regex> = LazyLock::new(|| re("정오"));
static DUR_HOURS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?P<h>[0-9]{1,2}|한|두|세|네|다섯|여섯)\s*시간(?:\s*(?P<half>반)|\s*(?P<m>[0-5]?[0-9])\s*분)?(?:\s*동안)?")
});
static DUR_MINS: LazyLock<Regex> = LazyLock::new(|| re(r"(?P<m>[0-9]{1,3})\s*분(?:\s*동안)?"));

fn is_hangul(c: char) -> bool {
    ('\u{AC00}'..='\u{D7A3}').contains(&c)
}

fn native(s: &str) -> Option<i64> {
    Some(match s {
        "한" | "하루" => 1,
        "두" | "이틀" => 2,
        "세" | "사흘" => 3,
        "네" | "나흘" => 4,
        "다섯" | "닷새" => 5,
        "여섯" | "엿새" => 6,
        "일곱" | "이레" => 7,
        "여덟" => 8,
        "아홉" => 9,
        "열" | "열흘" => 10,
        "열한" => 11,
        "열두" => 12,
        _ => return s.parse().ok(),
    })
}

fn num(c: &Captures, name: &str) -> Option<u32> {
    c.name(name).and_then(|m| m.as_str().parse().ok())
}

fn link_of(c: &Captures) -> Link {
    match c.name("link").map(|m| m.as_str()) {
        Some("부터") => Link::From,
        Some("까지") => Link::To,
        _ => Link::None,
    }
}

fn period_of(c: &Captures) -> Option<Period> {
    c.name("p").map(|m| match m.as_str() {
        "오전" | "아침" | "새벽" => Period::Am,
        "낮" => Period::Day,
        "밤" => Period::Night,
        _ => Period::Pm, // 오후·저녁
    })
}

fn week_offset(q: &str) -> i64 {
    match q {
        "다음" | "담" => 1,
        "다다음" => 2,
        "지난" | "저번" => -1,
        _ => 0, // 이번·이
    }
}

/// 조각 앞뒤가 단어 한가운데가 아닌지. 숫자로 시작하면 앞이 숫자·구분자가 아니어야 하고(«123시» 의 «23시» 가 아니게),
/// 한글로 시작하면 앞이 한글이 아니어야 한다(«대한 시민» 의 «한 시» 가 아니게).
fn prev_ok(s: &str, start: usize) -> bool {
    let (Some(first), Some(prev)) = (s[start..].chars().next(), s[..start].chars().next_back()) else {
        return true;
    };
    if first.is_ascii_digit() {
        !(prev.is_ascii_digit() || matches!(prev, '.' | ':' | '/'))
    } else if is_hangul(first) {
        !is_hangul(prev)
    } else {
        true
    }
}

fn next_char(s: &str, end: usize) -> Option<char> {
    s[end..].chars().next()
}

impl Scan {
    fn new(orig: &str) -> Self {
        Scan {
            masked: orig.to_string(),
            dates: vec![],
            times: vec![],
            durs: vec![],
            rels: vec![],
            repeat: vec![],
        }
    }

    /// 규칙 하나를 가려진 글에 돌린다. `f` 가 None 이면 그 자리는 조각이 아니다(가리지 않는다).
    fn pass(&mut self, re: &Regex, mut f: impl FnMut(&Captures, &str, &mut Self) -> bool) {
        let snapshot = self.masked.clone();
        for c in re.captures_iter(&snapshot) {
            let m = c.get(0).expect("전체 일치");
            if m.as_str().trim().is_empty() || !prev_ok(&snapshot, m.start()) {
                continue;
            }
            if f(&c, &snapshot, self) {
                if c.name("link").is_some_and(|l| l.as_str() == "마다") {
                    self.repeat.push(m.range());
                }
                self.masked.replace_range(m.range(), &" ".repeat(m.len()));
            }
        }
    }

    fn date(&mut self, c: &Captures, v: DateTok) -> bool {
        let span = c.get(0).expect("전체 일치").range();
        self.dates.push(Tok { v, span, link: link_of(c) });
        true
    }

    fn time(&mut self, c: &Captures, v: TimeTok) -> bool {
        let span = c.get(0).expect("전체 일치").range();
        self.times.push(Tok { v, span, link: link_of(c) });
        true
    }

    fn run(&mut self) {
        // 순서가 뜻이다: 긴 표현·«뒤» 가 붙은 표현을 먼저 먹어야 «3일 뒤» 의 «3일» 이 날짜가 되지 않는다.
        self.pass(&REL_TIME, |c, _, s| {
            let Some(n) = c.name("n").and_then(|m| native(m.as_str())) else { return false };
            let half = if c.name("half").is_some() { 30 } else { 0 };
            let min = if &c["u"] == "시간" { n * 60 + half } else { n };
            s.rels.push(Tok { v: min, span: c.get(0).expect("전체 일치").range(), link: link_of(c) });
            true
        });
        self.pass(&REL_DAYS, |c, _, s| {
            let (n, u) = (&c["n"], &c["u"]);
            let v = match (n, u) {
                ("일", "주일") => DateTok::Days(7),
                ("일", _) => return false,
                _ => {
                    let Some(k) = native(n) else { return false };
                    match u {
                        "일" => DateTok::Days(k),
                        "주" | "주일" => DateTok::Days(k * 7),
                        _ => DateTok::Months(k as u32), // 달·개월
                    }
                }
            };
            s.date(c, v)
        });
        self.pass(&NATIVE_DAYS, |c, _, s| {
            let Some(k) = native(&c["w"]) else { return false };
            s.date(c, DateTok::Days(k))
        });
        self.pass(&DATE_WORD, |c, _, s| {
            let k = match &c["w"] {
                "그저께" | "그제" => -2,
                "어제" => -1,
                "오늘" => 0,
                "내일" | "낼" => 1,
                "모레" | "내일모레" | "낼모레" => 2,
                _ => 3, // 글피
            };
            s.date(c, DateTok::Days(k))
        });
        self.pass(&YMD_SEP, |c, _, s| {
            let (Some(y), Some(m), Some(d)) = (num(c, "y"), num(c, "m"), num(c, "d")) else { return false };
            s.date(c, DateTok::Ymd { y: Some(y as i32), m: Some(m), d })
        });
        self.pass(&YMD_KO, |c, _, s| {
            let (Some(m), Some(d)) = (num(c, "m"), num(c, "d")) else { return false };
            s.date(c, DateTok::Ymd { y: num(c, "y").map(|y| y as i32), m: Some(m), d })
        });
        // «10/3» «10.3» — 숫자·글자가 바로 이어지면(«1.5시간», «2.5km», «1/2쯤») 날짜가 아니다.
        for rx in [&*MD_SLASH, &*MD_DOT] {
            self.pass(rx, |c, snap, s| {
                let end = c.get(0).expect("전체 일치").end();
                if next_char(snap, end).is_some_and(|n| n.is_alphanumeric() || matches!(n, '/' | '.' | ':')) {
                    return false;
                }
                let (Some(m), Some(d)) = (num(c, "m"), num(c, "d")) else { return false };
                // 달·날 범위 밖이면 날짜 표기가 아니라 그냥 수다(«3.50») — 틀린 날짜로 보고하지 않는다.
                if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                    return false;
                }
                s.date(c, DateTok::Ymd { y: None, m: Some(m), d })
            });
        }
        self.pass(&MONTH_DAY, |c, _, s| {
            let Some(d) = num(c, "d") else { return false };
            s.date(c, DateTok::MonthDay { months: week_offset(&c["q"]) as i32, d })
        });
        self.pass(&WEEKEND, |c, _, s| {
            let weeks = c.name("q").map_or(0, |q| week_offset(q.as_str()));
            s.date(c, DateTok::Weekend { weeks })
        });
        self.pass(&WEEKDAY, |c, snap, s| {
            let q = c.name("q").map(|m| m.as_str());
            let (wk, yo) = (c.name("wk").is_some(), c.name("yo").is_some());
            if !yo {
                // 글자 하나짜리 요일은 «다음 주 화» 처럼 «주» 뒤에서만, 그리고 뒤에 다른 글자가 안 붙을 때만 —
                // «이번 주 일정», «다음 주 수업» 의 «일»·«수» 는 요일이 아니다.
                let end = c.get(0).expect("전체 일치").end();
                if !wk || c.name("link").is_some() || next_char(snap, end).is_some_and(is_hangul) {
                    return false;
                }
            }
            let wd = match &c["wd"] {
                "월" => Weekday::Mon,
                "화" => Weekday::Tue,
                "수" => Weekday::Wed,
                "목" => Weekday::Thu,
                "금" => Weekday::Fri,
                "토" => Weekday::Sat,
                _ => Weekday::Sun,
            };
            let weeks = match q {
                Some("매") => {
                    s.repeat.push(c.get(0).expect("전체 일치").range());
                    None
                }
                // «이번 금요일» = 가장 가까운 금요일. «이번 주 금요일» = 이번 주(월~일)의 금요일.
                Some("이번") if !wk => None,
                Some(q) => Some(week_offset(q)),
                None => None,
            };
            s.date(c, DateTok::Weekday { weeks, wd })
        });
        self.pass(&BARE_DAY, |c, snap, s| {
            // «3일간 출장», «3일째», «3일치» — 기간이지 날짜가 아니다.
            let end = c.get(0).expect("전체 일치").end();
            if c.name("link").is_none() && next_char(snap, end).is_some_and(|n| "간째치차동".contains(n)) {
                return false;
            }
            let Some(d) = num(c, "d") else { return false };
            s.date(c, DateTok::Ymd { y: None, m: None, d })
        });
        self.pass(&REPEAT, |c, _, s| {
            s.repeat.push(c.get(0).expect("전체 일치").range());
            true
        });

        // «2시간» 의 «2시» 는 시각이 아니다 — 시 바로 뒤가 «간» 이면 거른다.
        let not_hours = |c: &Captures, snap: &str| {
            let end = c.get(0).expect("전체 일치").end();
            next_char(snap, end) != Some('간')
        };
        self.pass(&RANGE_SHORT, |c, snap, s| {
            if !not_hours(c, snap) {
                return false;
            }
            let (Some(h1), Some(h2)) = (num(c, "h1"), num(c, "h2")) else { return false };
            let whole = c.get(0).expect("전체 일치").range();
            let h1_end = c.name("h1").expect("h1").end();
            let p = period_of(c);
            // 둘로 쪼개 넣는다 — 엮는 규칙(물려받기·넘김)을 «3시~5시» 와 똑같이 타게.
            s.times.push(Tok {
                v: TimeTok { period: p, h: h1, m: 0, fixed: fixed(&c["h1"], h1) },
                span: whole.start..h1_end,
                link: Link::From,
            });
            s.times.push(Tok {
                v: TimeTok { period: None, h: h2, m: 0, fixed: fixed(&c["h2"], h2) },
                span: h1_end..whole.end,
                link: link_of(c),
            });
            true
        });
        self.pass(&CLOCK, |c, snap, s| {
            let end = c.get(0).expect("전체 일치").end();
            if next_char(snap, end).is_some_and(|n| n.is_ascii_digit()) {
                return false;
            }
            let (Some(h), Some(m)) = (num(c, "h"), num(c, "m")) else { return false };
            s.time(c, TimeTok { period: period_of(c), h, m, fixed: fixed(&c["h"], h) })
        });
        self.pass(&HOUR, |c, snap, s| {
            if !not_hours(c, snap) {
                return false;
            }
            let Some(h) = native(&c["h"]).map(|h| h as u32) else { return false };
            let m = if c.name("half").is_some() { 30 } else { num(c, "m").unwrap_or(0) };
            s.time(c, TimeTok { period: period_of(c), h, m, fixed: fixed(&c["h"], h) })
        });
        self.pass(&NOON, |c, _, s| s.time(c, TimeTok { period: None, h: 12, m: 0, fixed: true }));

        self.pass(&DUR_HOURS, |c, _, s| {
            let Some(h) = native(&c["h"]) else { return false };
            let m = if c.name("half").is_some() { 30 } else { num(c, "m").unwrap_or(0) as i64 };
            s.durs.push(Tok { v: h * 60 + m, span: c.get(0).expect("전체 일치").range(), link: Link::None });
            true
        });
        self.pass(&DUR_MINS, |c, _, s| {
            let Some(m) = num(c, "m") else { return false };
            s.durs.push(Tok { v: m as i64, span: c.get(0).expect("전체 일치").range(), link: Link::None });
            true
        });

        self.dates.sort_by_key(|t| t.span.start);
        self.times.sort_by_key(|t| t.span.start);
    }
}

/// «08:30», «09시», «15시» = 24시간제로 적은 것 — 오후로 굴리지 않는다.
fn fixed(raw: &str, h: u32) -> bool {
    raw.starts_with('0') || h >= 13 || h == 0
}

// ───────────────────────── 풀기 ─────────────────────────

/// 날짜 조각 → 날짜. `anchor` = 범위의 시작(끝 조각은 그 뒤로 굴린다). 없는 날이면 None.
fn resolve_date(t: DateTok, today: NaiveDate, anchor: Option<NaiveDate>) -> Option<NaiveDate> {
    let from = anchor.unwrap_or(today);
    match t {
        DateTok::Days(k) => today.checked_add_signed(Duration::days(k)),
        DateTok::Months(k) => today.checked_add_months(Months::new(k)),
        DateTok::Ymd { y: Some(y), m: Some(m), d } => NaiveDate::from_ymd_opt(y, m, d),
        DateTok::Ymd { y: None, m: Some(m), d } => {
            // 올해부터 앞으로 — 2월 29일은 다음 윤년까지 간다(최대 8년).
            (from.year()..from.year() + 9)
                .filter_map(|y| NaiveDate::from_ymd_opt(y, m, d))
                .find(|&dt| dt >= from)
        }
        DateTok::Ymd { y: None, m: None, d } => {
            // 이번 달부터 앞으로 — «31일» 이 30일까지인 달이면 다음 달로.
            let first = from.with_day(1)?;
            (0..13)
                .filter_map(|k| first.checked_add_months(Months::new(k))?.with_day(d))
                .find(|&dt| dt >= from)
        }
        DateTok::Ymd { y: Some(_), m: None, .. } => None,
        DateTok::MonthDay { months, d } => {
            let first = today.with_day(1)?;
            let first = if months >= 0 {
                first.checked_add_months(Months::new(months as u32))?
            } else {
                first.checked_sub_months(Months::new(months.unsigned_abs()))?
            };
            first.with_day(d)
        }
        DateTok::Weekday { weeks: Some(k), wd } => {
            let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
            monday.checked_add_signed(Duration::days(k * 7 + wd.num_days_from_monday() as i64))
        }
        DateTok::Weekday { weeks: None, wd } => {
            let ahead = (7 + wd.num_days_from_monday() - from.weekday().num_days_from_monday()) % 7;
            from.checked_add_days(Days::new(ahead as u64))
        }
        DateTok::Weekend { weeks } => {
            let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
            monday.checked_add_signed(Duration::days(weeks * 7 + 5))
        }
    }
}

/// 시각 조각 → (시각, 다음 날로 넘어가나). 없는 시각이면 None.
fn resolve_time(t: TimeTok, inherit: Option<Period>) -> Option<(NaiveTime, bool)> {
    if t.m > 59 || t.h > 23 {
        return None;
    }
    let (h, next) = match (t.period.or(if t.fixed { None } else { inherit }), t.h) {
        (_, h) if t.fixed && t.period.is_none() => (h, false),
        (Some(Period::Am), 12) => (0, false),
        (Some(Period::Am), h) if h <= 11 => (h, false),
        (Some(Period::Pm), h @ 1..=11) => (h + 12, false),
        (Some(Period::Pm), h) if h >= 12 => (h, false), // 오후 12시 = 정오, «오후 15시» 도 받아 준다
        (Some(Period::Day), h @ 1..=6) => (h + 12, false),
        (Some(Period::Day), h @ 7..=12) => (h, false),
        (Some(Period::Night), h @ 6..=11) => (h + 12, false),
        (Some(Period::Night), 12) => (0, true),
        (Some(Period::Night), h @ 1..=5) => (h, true),
        // 오전/오후를 안 적은 1~6시는 오후 — «3시 치과» 를 새벽 3시로 넣는 캘린더는 없다.
        (None, h @ 1..=6) => (h + 12, false),
        (None, h) if h <= 12 => (h, false),
        _ => return None,
    };
    Some((NaiveTime::from_hms_opt(h, t.m, 0)?, next))
}

/// 두 조각이 범위로 이어지나 — 사이(다른 조각은 빼고)가 «~» 뿐이거나, 앞 조각·사이 조각이 «부터» 로 끝나거나, 뒤 조각이 «까지» 로 끝나면.
fn connected(orig: &str, a: &Range<usize>, b: &Range<usize>, a_link: Link, b_link: Link, others: &[(Range<usize>, Link)]) -> bool {
    if a.end > b.start {
        return false;
    }
    let mut gap = String::new();
    let mut from_between = false;
    for (i, ch) in orig[a.end..b.start].char_indices() {
        let at = a.end + i;
        match others.iter().find(|(r, _)| r.contains(&at)) {
            Some((r, l)) => {
                from_between |= *l == Link::From && r.start >= a.end && r.end <= b.start;
            }
            None => gap.push(ch),
        }
    }
    match gap.trim() {
        "~" | "-" | "–" | "〜" | "부터" => true,
        "" => a_link == Link::From || from_between || b_link == Link::To,
        _ => false,
    }
}

/// 범위 두 조각 사이에서 다른 조각이 아닌 자리(«~»·«부터» 같은 이음표) — 제목에서 같이 걷어 낸다.
fn bridge(a: &Range<usize>, b: &Range<usize>, others: &[(Range<usize>, Link)]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = vec![];
    for at in a.end..b.start {
        if others.iter().any(|(r, _)| r.contains(&at)) {
            continue;
        }
        match out.last_mut() {
            Some(r) if r.end == at => r.end = at + 1,
            _ => out.push(at..at + 1),
        }
    }
    out
}

fn warn_invalid(w: &mut Vec<String>, orig: &str, span: &Range<usize>, what: &str) {
    w.push(format!("「{}」 — 없는 {what}예요", orig[span.clone()].trim()));
}

pub fn parse(text: &str, now: NaiveDateTime, base: Option<NaiveDate>) -> Draft {
    let mut s = Scan::new(text);
    s.run();
    let today = now.date();
    let mut warnings = vec![];
    let mut used: Vec<Range<usize>> = vec![];
    let mut invalid = false;

    // 날짜 — 첫 번째 쓸 수 있는 것이 시작, 바로 이어진 다음 것이 끝.
    let others: Vec<(Range<usize>, Link)> = s
        .dates
        .iter()
        .map(|t| (t.span.clone(), t.link))
        .chain(s.times.iter().map(|t| (t.span.clone(), t.link)))
        .collect();
    let mut start_date = None;
    let mut end_date = None;
    let mut weekend = false;
    for (i, t) in s.dates.iter().enumerate() {
        let Some(d) = resolve_date(t.v, today, None) else {
            warn_invalid(&mut warnings, text, &t.span, "날짜");
            invalid = true;
            continue;
        };
        used.push(t.span.clone());
        start_date = Some(d);
        weekend = matches!(t.v, DateTok::Weekend { .. });
        if let Some(n) = s.dates.get(i + 1) {
            if connected(text, &t.span, &n.span, t.link, n.link, &others) {
                match resolve_date(n.v, today, Some(d)) {
                    Some(e) if e >= d => {
                        end_date = Some(e);
                        used.push(n.span.clone());
                        used.extend(bridge(&t.span, &n.span, &others));
                    }
                    Some(_) => warnings.push("끝나는 날이 시작보다 앞이라 끝은 뺐어요".into()),
                    None => {
                        warn_invalid(&mut warnings, text, &n.span, "날짜");
                        invalid = true;
                    }
                }
            }
        }
        break;
    }

    // 시각 — 같은 모양. 끝은 시작의 오전/오후를 물려받는다(«오후 3시~5시»).
    let mut start_time = None;
    let mut end_time = None;
    for (i, t) in s.times.iter().enumerate() {
        let Some(st) = resolve_time(t.v, None) else {
            warn_invalid(&mut warnings, text, &t.span, "시각");
            invalid = true;
            continue;
        };
        start_time = Some((t, st));
        if let Some(n) = s.times.get(i + 1) {
            if connected(text, &t.span, &n.span, t.link, n.link, &others) {
                match resolve_time(n.v, t.v.period) {
                    Some(et) => end_time = Some((n, et)),
                    None => {
                        warn_invalid(&mut warnings, text, &n.span, "시각");
                        invalid = true;
                    }
                }
            }
        }
        break;
    }

    // 날짜가 범위인데 시각이 하나뿐이면 어디에 붙일지 모른다 — 종일 범위로 두고 시각은 제목에 남긴다.
    if end_date.is_some() && start_time.is_some() && end_time.is_none() {
        start_time = None;
        warnings.push("시각을 어느 날에 붙일지 몰라 종일로 두었어요".into());
    }
    // 주말에 시각이 붙으면 토요일 하루.
    if weekend && end_date.is_none() && start_time.is_none() {
        end_date = start_date.and_then(|d| d.checked_add_days(Days::new(1)));
    }

    let mut draft = Draft {
        title: String::new(),
        all_day: false,
        start_date: None,
        start_time: None,
        end_date: None,
        end_time: None,
        warnings: vec![],
        miss: None,
    };

    let fmt_d = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let fmt_t = |t: NaiveDateTime| t.format("%H:%M").to_string();

    if let Some((st_tok, (st, st_next))) = start_time {
        used.push(st_tok.span.clone());
        // 날짜가 없으면: 보고 있는 날(오늘이 아니면), 아니면 오늘 — 오늘 이미 지난 시각이면 내일.
        let explicit = start_date.is_some();
        let day = start_date.or(base).unwrap_or(today);
        let mut start = day.and_time(st);
        if st_next {
            start += Duration::days(1);
        }
        if !explicit && base.is_none() && start < now {
            start += Duration::days(1);
        }
        let end = match end_time {
            Some((et_tok, (et, et_next))) => {
                used.push(et_tok.span.clone());
                used.extend(bridge(&st_tok.span, &et_tok.span, &others));
                let mut end = end_date.unwrap_or(start.date()).and_time(et);
                // 끝도 «다음 날 새벽» 이면 하루 넘긴다 — 단, 시작이 이미 넘어갔으면(«밤 12시~1시») 같은 날 안이다. 두 번 넘기면 25시간이 된다(코덱스 개발 6).
                if et_next && !st_next && end_date.is_none() {
                    end += Duration::days(1);
                }
                if end_date.is_none() {
                    // «11시~1시» = 11시~13시, «밤 11시~1시» = 다음 날 1시.
                    let own = et_tok.v.period.is_some() || et_tok.v.fixed;
                    if end <= start && !own && et.hour() < 12 {
                        end += Duration::hours(12);
                    }
                    if end <= start {
                        end += Duration::days(1);
                    }
                }
                if end < start {
                    warnings.push("끝나는 시각이 시작보다 앞이라 한 시간으로 잡았어요".into());
                    start + Duration::hours(1)
                } else {
                    end
                }
            }
            None => match s.durs.first() {
                Some(d) if d.v > 0 && d.v <= 24 * 60 => {
                    used.push(d.span.clone());
                    start + Duration::minutes(d.v)
                }
                _ => start + Duration::hours(1),
            },
        };
        draft.start_date = Some(fmt_d(start.date()));
        draft.start_time = Some(fmt_t(start));
        draft.end_date = Some(fmt_d(end.date()));
        draft.end_time = Some(fmt_t(end));
    } else if let Some(d) = start_date {
        draft.all_day = true;
        draft.start_date = Some(fmt_d(d));
        draft.end_date = Some(fmt_d(end_date.unwrap_or(d)));
    } else if let Some(r) = s.rels.first() {
        // «30분 뒤 전화» — 분 단위로 올린다(10:00:20 에 «30분 뒤» = 10:31 이 아니라 10:30 이 되게 초를 버리고).
        used.push(r.span.clone());
        let start = now.with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or(now) + Duration::minutes(r.v);
        let end = start + Duration::hours(1);
        draft.start_date = Some(fmt_d(start.date()));
        draft.start_time = Some(fmt_t(start));
        draft.end_date = Some(fmt_d(end.date()));
        draft.end_time = Some(fmt_t(end));
    }

    if !s.repeat.is_empty() {
        used.extend(s.repeat.iter().cloned());
        warnings.push("반복은 아직 못 만들어요 — 첫 번 한 번만 넣어요".into());
    }

    // 쓰지 않은 날짜·시각이 남았으면(«3시 말고 5시», «3시 또는 5시») 어느 쪽인지 모른다 — 첫 것으로 잡되 사람이 보게.
    // 경고가 없으면 맥은 ↩ 로, iOS Siri 는 화면 없이 바로 넣는다(코덱스 개발 14 P1). 못 읽는 조각은 위에서 이미 경고했다.
    if warnings.is_empty() {
        let inside = |sp: &Range<usize>| used.iter().any(|u| u.start <= sp.start && sp.end <= u.end);
        let left = s.dates.iter().any(|t| !inside(&t.span) && resolve_date(t.v, today, None).is_some())
            || s.times.iter().any(|t| !inside(&t.span) && resolve_time(t.v, None).is_some());
        if left {
            warnings.push("날짜나 시각이 여럿이라 첫 것으로 잡았어요 — 맞는지 봐 주세요".into());
        }
    }

    draft.title = title_without(text, &used);
    draft.miss = if text.trim().is_empty() {
        None
    } else if !s.repeat.is_empty() {
        Some(Miss::Repeat)
    } else if invalid {
        Some(Miss::Invalid)
    } else if draft.start_date.is_none() {
        Some(Miss::NoDate)
    } else {
        None
    };
    draft.warnings = warnings;
    draft
}

/// 쓴 조각을 걷어 낸 나머지 = 제목. 공백을 하나로, 가장자리의 이음표(«~», «-», «,»)는 뗀다.
fn title_without(text: &str, used: &[Range<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, ch) in text.char_indices() {
        out.push(if used.iter().any(|r| r.contains(&i)) { ' ' } else { ch });
    }
    let joined = out.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '~' | '-' | '–' | '〜' | ',' | '·'))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 기준: 2026-09-29 화요일 오전 10시.
    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, 29).unwrap().and_hms_opt(10, 0, 0).unwrap()
    }

    fn p(text: &str) -> Draft {
        parse(text, now(), None)
    }

    /// 시각 일정: (시작일, 시작, 끝일, 끝, 제목).
    #[track_caller]
    fn timed(text: &str, sd: &str, st: &str, ed: &str, et: &str, title: &str) {
        let d = p(text);
        assert_eq!(
            (d.all_day, d.start_date.as_deref(), d.start_time.as_deref(), d.end_date.as_deref(), d.end_time.as_deref(), d.title.as_str()),
            (false, Some(sd), Some(st), Some(ed), Some(et), title),
            "«{text}» → {d:?}"
        );
    }

    /// 종일 일정: (시작일, 끝일(포함), 제목).
    #[track_caller]
    fn all_day(text: &str, sd: &str, ed: &str, title: &str) {
        let d = p(text);
        assert_eq!(
            (d.all_day, d.start_date.as_deref(), d.end_date.as_deref(), d.start_time.as_deref(), d.title.as_str()),
            (true, Some(sd), Some(ed), None, title),
            "«{text}» → {d:?}"
        );
    }

    #[track_caller]
    fn no_date(text: &str, title: &str, miss: Option<Miss>) {
        let d = p(text);
        assert_eq!((d.start_date.as_deref(), d.title.as_str(), d.miss), (None, title, miss), "«{text}» → {d:?}");
    }

    // ── 상대 날짜 낱말 ──
    #[test]
    fn relative_words() {
        timed("내일 3시 치과", "2026-09-30", "15:00", "2026-09-30", "16:00", "치과");
        timed("오늘 오후 2시 회의", "2026-09-29", "14:00", "2026-09-29", "15:00", "회의");
        timed("모레 10시 반 미팅", "2026-10-01", "10:30", "2026-10-01", "11:30", "미팅");
        all_day("글피 이사", "2026-10-02", "2026-10-02", "이사");
        timed("내일모레 저녁 7시 저녁 약속", "2026-10-01", "19:00", "2026-10-01", "20:00", "저녁 약속");
        all_day("어제 산책", "2026-09-28", "2026-09-28", "산책");
        all_day("그저께 회식", "2026-09-27", "2026-09-27", "회식");
        all_day("낼 치과", "2026-09-30", "2026-09-30", "치과");
        timed("내일3시치과", "2026-09-30", "15:00", "2026-09-30", "16:00", "치과");
        timed("치과 내일 오후 3시에", "2026-09-30", "15:00", "2026-09-30", "16:00", "치과");
        all_day("오늘은 쉬는 날", "2026-09-29", "2026-09-29", "쉬는 날");
    }

    // ── N일·주·달 뒤 ──
    #[test]
    fn relative_offsets() {
        all_day("3일 뒤 병원", "2026-10-02", "2026-10-02", "병원");
        all_day("2주 후 발표", "2026-10-13", "2026-10-13", "발표");
        all_day("한 달 뒤 정산", "2026-10-29", "2026-10-29", "정산");
        all_day("일주일 뒤 마감", "2026-10-06", "2026-10-06", "마감");
        all_day("이틀 뒤 이사", "2026-10-01", "2026-10-01", "이사");
        all_day("3개월 후 건강검진", "2026-12-29", "2026-12-29", "건강검진");
        timed("30분 뒤 전화", "2026-09-29", "10:30", "2026-09-29", "11:30", "전화");
        timed("2시간 후 회의", "2026-09-29", "12:00", "2026-09-29", "13:00", "회의");
        timed("1시간 반 뒤 출발", "2026-09-29", "11:30", "2026-09-29", "12:30", "출발");
    }

    // ── 요일 (월요일 시작 주) ──
    #[test]
    fn weekdays() {
        timed("다음 주 화요일 15시 회의", "2026-10-06", "15:00", "2026-10-06", "16:00", "회의");
        timed("이번 주 금요일 저녁 7시 회식", "2026-10-02", "19:00", "2026-10-02", "20:00", "회식");
        all_day("이번 주 일요일 등산", "2026-10-04", "2026-10-04", "등산");
        all_day("금요일 치과", "2026-10-02", "2026-10-02", "치과");
        timed("화요일 3시 상담", "2026-09-29", "15:00", "2026-09-29", "16:00", "상담");
        all_day("다다음 주 월요일 보고", "2026-10-12", "2026-10-12", "보고");
        all_day("지난주 금요일 회고", "2026-09-25", "2026-09-25", "회고");
        timed("담주 수 2시 면접", "2026-10-07", "14:00", "2026-10-07", "15:00", "면접");
        all_day("다음 금요일 마감", "2026-10-09", "2026-10-09", "마감");
        all_day("이번 월요일 휴가", "2026-10-05", "2026-10-05", "휴가");
        all_day("금요일까지 보고서", "2026-10-02", "2026-10-02", "보고서");
    }

    // ── 절대 날짜 ──
    #[test]
    fn absolute_dates() {
        all_day("10월 3일 개천절", "2026-10-03", "2026-10-03", "개천절");
        all_day("10/9 한글날", "2026-10-09", "2026-10-09", "한글날");
        all_day("2026-12-25 크리스마스", "2026-12-25", "2026-12-25", "크리스마스");
        all_day("2027년 1월 1일 새해", "2027-01-01", "2027-01-01", "새해");
        all_day("2027.3.1 삼일절", "2027-03-01", "2027-03-01", "삼일절");
        all_day("10.3 개천절", "2026-10-03", "2026-10-03", "개천절");
        all_day("3월 1일 삼일절", "2027-03-01", "2027-03-01", "삼일절");
        all_day("9월 28일 지난 일", "2027-09-28", "2027-09-28", "지난 일");
        all_day("15일 월급날", "2026-10-15", "2026-10-15", "월급날");
        all_day("30일 마감", "2026-09-30", "2026-09-30", "마감");
        all_day("31일 결산", "2026-10-31", "2026-10-31", "결산");
        all_day("다음 달 3일 정기검진", "2026-10-03", "2026-10-03", "정기검진");
        all_day("지난달 15일 정산", "2026-08-15", "2026-08-15", "정산");
        all_day("2월 29일 윤일", "2028-02-29", "2028-02-29", "윤일");
    }

    // ── 시각 ──
    #[test]
    fn times() {
        timed("3시 치과", "2026-09-29", "15:00", "2026-09-29", "16:00", "치과");
        timed("11시 커피", "2026-09-29", "11:00", "2026-09-29", "12:00", "커피");
        // 날짜 없이 이미 지난 시각 → 내일.
        timed("9시 스탠드업", "2026-09-30", "09:00", "2026-09-30", "10:00", "스탠드업");
        timed("08:30 조깅", "2026-09-30", "08:30", "2026-09-30", "09:30", "조깅");
        timed("새벽 2시 경기 보기", "2026-09-30", "02:00", "2026-09-30", "03:00", "경기 보기");
        // «오늘» 이라고 적었으면 지났어도 오늘.
        timed("오늘 9시 회의", "2026-09-29", "09:00", "2026-09-29", "10:00", "회의");
        timed("정오 점심", "2026-09-29", "12:00", "2026-09-29", "13:00", "점심");
        timed("오후 12시 점심", "2026-09-29", "12:00", "2026-09-29", "13:00", "점심");
        timed("오전 12시 30분 배포", "2026-09-30", "00:30", "2026-09-30", "01:30", "배포");
        timed("밤 12시 마감", "2026-09-30", "00:00", "2026-09-30", "01:00", "마감");
        timed("낮 1시 약속", "2026-09-29", "13:00", "2026-09-29", "14:00", "약속");
        timed("저녁 여섯시 반 요가", "2026-09-29", "18:30", "2026-09-29", "19:30", "요가");
        timed("두시 약속", "2026-09-29", "14:00", "2026-09-29", "15:00", "약속");
        timed("3시 30분 회의", "2026-09-29", "15:30", "2026-09-29", "16:30", "회의");
        timed("15:30 회의", "2026-09-29", "15:30", "2026-09-29", "16:30", "회의");
        timed("4시 반에 전화", "2026-09-29", "16:30", "2026-09-29", "17:30", "전화");
    }

    // ── 범위·길이 ──
    #[test]
    fn ranges_and_durations() {
        timed("3시부터 5시까지 워크숍", "2026-09-29", "15:00", "2026-09-29", "17:00", "워크숍");
        timed("내일 3-5시 세미나", "2026-09-30", "15:00", "2026-09-30", "17:00", "세미나");
        timed("오후 3시~5시 반 스터디", "2026-09-29", "15:00", "2026-09-29", "17:30", "스터디");
        timed("14:00-15:30 리뷰", "2026-09-29", "14:00", "2026-09-29", "15:30", "리뷰");
        timed("내일 11시~1시 점심", "2026-09-30", "11:00", "2026-09-30", "13:00", "점심");
        timed("내일 오전 10시~2시 행사", "2026-09-30", "10:00", "2026-09-30", "14:00", "행사");
        timed("밤 11시~1시 배포", "2026-09-29", "23:00", "2026-09-30", "01:00", "배포");
        timed("오늘 밤 12시~1시 배포", "2026-09-30", "00:00", "2026-09-30", "01:00", "배포");
        timed("밤 12시부터 새벽 2시까지 점검", "2026-09-30", "00:00", "2026-09-30", "02:00", "점검");
        timed("내일 3시 2시간 회의", "2026-09-30", "15:00", "2026-09-30", "17:00", "회의");
        timed("내일 3시 90분 상담", "2026-09-30", "15:00", "2026-09-30", "16:30", "상담");
        all_day("10월 3일부터 5일까지 여행", "2026-10-03", "2026-10-05", "여행");
        all_day("10/3~10/5 여행", "2026-10-03", "2026-10-05", "여행");
        all_day("내일부터 모레까지 워크숍", "2026-09-30", "2026-10-01", "워크숍");
        all_day("금요일부터 일요일까지 캠핑", "2026-10-02", "2026-10-04", "캠핑");
        all_day("이번 주말 캠핑", "2026-10-03", "2026-10-04", "캠핑");
        timed("다음 주말 3시 결혼식", "2026-10-10", "15:00", "2026-10-10", "16:00", "결혼식");
        timed("내일 3시부터 모레 5시까지 학회", "2026-09-30", "15:00", "2026-10-01", "17:00", "학회");
    }

    // ── 반복 — 알아듣되 한 번만 ──
    #[test]
    fn repeats_are_flagged() {
        let d = p("매주 월 9시 브리핑");
        assert_eq!((d.start_date.as_deref(), d.start_time.as_deref(), d.title.as_str()), (Some("2026-10-05"), Some("09:00"), "브리핑"));
        assert_eq!(d.miss, Some(Miss::Repeat));
        assert_eq!(d.warnings.len(), 1);
        let d = p("매일 9시 운동");
        assert_eq!((d.start_date.as_deref(), d.title.as_str(), d.miss), (Some("2026-09-30"), "운동", Some(Miss::Repeat)));
        let d = p("월요일마다 주간회의");
        assert_eq!((d.start_date.as_deref(), d.title.as_str(), d.miss), (Some("2026-10-05"), "주간회의", Some(Miss::Repeat)));
    }

    // ── 날짜가 아닌 것은 건드리지 않는다 ──
    #[test]
    fn leaves_non_dates_alone() {
        no_date("치과", "치과", Some(Miss::NoDate));
        no_date("", "", None);
        no_date("이번 주 일정 정리", "이번 주 일정 정리", Some(Miss::NoDate));
        no_date("다음 주 수업 준비", "다음 주 수업 준비", Some(Miss::NoDate));
        no_date("월세 내기", "월세 내기", Some(Miss::NoDate));
        no_date("3시간 회의", "3시간 회의", Some(Miss::NoDate));
        no_date("1.5시간 달리기", "1.5시간 달리기", Some(Miss::NoDate));
        no_date("3일간 출장", "3일간 출장", Some(Miss::NoDate));
        no_date("대한 시민 모임", "대한 시민 모임", Some(Miss::NoDate));
        no_date("2.5km 달리기", "2.5km 달리기", Some(Miss::NoDate));
    }

    // ── 없는 값은 추측하지 않고 알린다 ──
    #[test]
    fn invalid_values_are_reported() {
        no_date("2월 30일 이상한 날", "2월 30일 이상한 날", Some(Miss::Invalid));
        assert_eq!(p("2월 30일 이상한 날").warnings, vec!["「2월 30일」 — 없는 날짜예요".to_string()]);
        no_date("25시 회의", "25시 회의", Some(Miss::Invalid));
        // 날짜는 맞고 시각만 틀리면 종일로 두고 알린다.
        let d = p("내일 25시 회의");
        assert_eq!((d.all_day, d.start_date.as_deref(), d.title.as_str(), d.miss), (true, Some("2026-09-30"), "25시 회의", Some(Miss::Invalid)));
        // 날짜 범위 + 시각 하나 — 어느 날에 붙일지 모른다. 종일로 두되 경고해 바로 넣지 않게.
        let d = p("10월 3일부터 5일까지 오후 3시 행사");
        assert_eq!((d.all_day, d.title.as_str(), d.warnings.len()), (true, "오후 3시 행사", 1));
        // 없는 날짜를 빼고 시각만 남아도 경고가 남는다(프론트가 시트로 연다).
        let d = p("2026-02-30 3시 회의");
        assert_eq!((d.miss, d.warnings.len()), (Some(Miss::Invalid), 1));
        let d = p("모레부터 내일까지 휴가");
        assert_eq!((d.start_date.as_deref(), d.end_date.as_deref()), (Some("2026-10-01"), Some("2026-10-01")));
        assert_eq!(d.warnings.len(), 1);
    }

    // ── 보고 있는 날(base) ──
    #[test]
    fn base_day_takes_dateless_times() {
        let base = NaiveDate::from_ymd_opt(2026, 10, 5);
        let d = parse("3시 치과", now(), base);
        assert_eq!((d.start_date.as_deref(), d.start_time.as_deref()), (Some("2026-10-05"), Some("15:00")));
        // 보고 있는 날에는 «지났으면 내일» 을 적용하지 않는다.
        let d = parse("8시 조깅", now(), base);
        assert_eq!((d.start_date.as_deref(), d.start_time.as_deref()), (Some("2026-10-05"), Some("08:00")));
        // 날짜를 적었으면 그게 이긴다.
        let d = parse("내일 3시 치과", now(), base);
        assert_eq!(d.start_date.as_deref(), Some("2026-09-30"));
    }

    // ── 해 넘김 ──
    #[test]
    fn year_boundary() {
        let eve = NaiveDate::from_ymd_opt(2026, 12, 30).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let d = parse("1월 2일 신년회", eve, None);
        assert_eq!(d.start_date.as_deref(), Some("2027-01-02"));
        let d = parse("다음 주 월요일 시무식", eve, None);
        assert_eq!(d.start_date.as_deref(), Some("2027-01-04"));
        let d = parse("12/31~1/2 휴가", eve, None);
        assert_eq!((d.start_date.as_deref(), d.end_date.as_deref(), d.title.as_str()), (Some("2026-12-31"), Some("2027-01-02"), "휴가"));
    }

    #[test]
    fn deterministic() {
        for text in ["내일 3시 치과", "매주 월 9시 브리핑", "2월 30일", "치과"] {
            assert_eq!(p(text), p(text));
        }
    }

    // ── 쓰지 않은 날짜·시각이 남으면 경고(코덱스 개발 14) — 경고가 없으면 Siri 가 화면 없이 넣는다 ──
    #[test]
    fn leftover_when_is_warned() {
        for t in ["내일 3시 말고 5시 치과", "3시 또는 5시 회의", "금요일 아니면 토요일 등산"] {
            let d = p(t);
            assert_eq!(d.warnings, vec!["날짜나 시각이 여럿이라 첫 것으로 잡았어요 — 맞는지 봐 주세요".to_string()], "«{t}» → {d:?}");
        }
        // 범위로 이어 쓴 건 하나다.
        assert!(p("내일 3시~5시 회의").warnings.is_empty());
        assert!(p("10월 3일부터 5일까지 여행").warnings.is_empty());
    }
}
