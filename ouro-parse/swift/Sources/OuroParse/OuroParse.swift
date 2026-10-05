// 러스트 파서를 감싼 스위프트 문. 날짜 계산은 하지 않는다 — 러스트가 준 «화면 값»(로컬 날짜·시각 글)을
// 이 기기의 달력으로 `Date` 에 옮기기만 한다(맥 프론트의 `quickToDraft`·`combine` 자리).

import Foundation
import OuroParseFFI

/// 러스트 `Draft` 그대로(camelCase JSON).
public struct QuickDraft: Decodable, Equatable, Sendable {
    public var title: String
    public var allDay: Bool
    /// 날짜를 못 찾았으면 넷 다 nil — 사람이 시트에서 고른다.
    public var startDate: String?
    public var startTime: String?
    /// 종일이면 **포함**(9/28 ~ 9/28 = 하루).
    public var endDate: String?
    public var endTime: String?
    public var warnings: [String]
    /// "no_date" · "invalid" · "repeat"
    public var miss: String?

    /// 바로 넣어도 되나 — 날짜를 알아들었고, 제목이 있고, 경고가 하나도 없을 때만(맥 `canDirect` 와 같다).
    public var canDirect: Bool { startDate != nil && !title.isEmpty && warnings.isEmpty }
}

/// 파서 결과를 이 기기 달력의 때로 옮긴 것.
public struct QuickWhen: Equatable, Sendable {
    public var allDay: Bool
    public var start: Date
    /// 시각 일정 = 끝 시각, 종일 = 마지막 날(포함)의 0시.
    public var end: Date
}

public enum QuickParse {
    static func posix(_ format: String, _ cal: Calendar) -> DateFormatter {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = cal.timeZone
        f.dateFormat = format
        return f
    }

    /// 「내일 3시 치과」 → 초안. `base` = 사람이 보고 있는 날(오늘이 아니면) — 날짜 없는 «3시 치과» 가 그날로 간다.
    /// nil 은 입력을 거절했을 때뿐이다 — 부르는 쪽은 «못 알아들음» 으로 다룬다.
    public static func parse(_ text: String, now: Date = Date(), base: Date? = nil, calendar: Calendar = .current) -> QuickDraft? {
        // C 글은 NUL 에서 끝난다 — 뒤를 몰래 잘라 «치과\0내일» 의 «내일» 을 잃느니 거절한다.
        if text.utf8.contains(0) { return nil }
        let nowS = posix("yyyy-MM-dd'T'HH:mm:ss", calendar).string(from: now)
        let baseS = base.map { posix("yyyy-MM-dd", calendar).string(from: $0) }
        let p: UnsafeMutablePointer<CChar>? = text.withCString { t in
            nowS.withCString { n in
                if let baseS { return baseS.withCString { b in ouro_parse(t, n, b) } }
                return ouro_parse(t, n, nil)
            }
        }
        guard let p else { return nil }
        defer { ouro_free(p) }
        return try? JSONDecoder().decode(QuickDraft.self, from: Data(String(cString: p).utf8))
    }

    /// 초안의 날짜·시각 → `Date`. 날짜가 없거나, 이 시간대에 **없는 시각**(서머타임으로 건너뛴 02:30 등)이면 nil —
    /// 맥 `combine` 처럼 조용히 한 시간 밀어 넣지 않는다(사람이 시트에서 고른다).
    public static func when(_ d: QuickDraft, calendar: Calendar = .current) -> QuickWhen? {
        guard let sd = d.startDate, let ed = d.endDate else { return nil }
        if d.allDay {
            let f = posix("yyyy-MM-dd", calendar)
            guard let s = f.date(from: sd), let e = f.date(from: ed) else { return nil }
            return QuickWhen(allDay: true, start: calendar.startOfDay(for: s), end: calendar.startOfDay(for: e))
        }
        guard let st = d.startTime, let et = d.endTime, let s = exact(sd, st, calendar), let e = exact(ed, et, calendar) else { return nil }
        return QuickWhen(allDay: false, start: s, end: e)
    }

    /// 벽시계 그대로인 때만 — 거꾸로 적어 같은 글이 나오지 않으면 없는 시각이다.
    static func exact(_ ymd: String, _ hm: String, _ cal: Calendar) -> Date? {
        let f = posix("yyyy-MM-dd HH:mm", cal)
        f.isLenient = false
        let s = "\(ymd) \(hm)"
        guard let d = f.date(from: s), f.string(from: d) == s else { return nil }
        return d
    }
}
