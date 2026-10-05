// 스위프트 → 러스트 왕복. 파서의 판단 자체는 러스트 테스트(`cargo test`, ouro-parse)가 지킨다 — 여기선 문이 맞게 열리는지만.

import Foundation
import Testing
@testable import OuroParse

private func cal(_ tz: String) -> Calendar {
    var c = Calendar(identifier: .gregorian)
    c.timeZone = TimeZone(identifier: tz)!
    return c
}

private func at(_ s: String, _ c: Calendar) -> Date {
    QuickParse.posix("yyyy-MM-dd HH:mm", c).date(from: s)!
}

@Test func tomorrowDentist() throws {
    let c = cal("Asia/Seoul")
    let d = try #require(QuickParse.parse("내일 3시 치과", now: at("2026-09-29 10:00", c), calendar: c))
    #expect(d.title == "치과")
    #expect(d.startDate == "2026-09-30" && d.startTime == "15:00" && d.endTime == "16:00")
    #expect(d.canDirect)
    let w = try #require(QuickParse.when(d, calendar: c))
    #expect(w.start == at("2026-09-30 15:00", c))
    #expect(w.end == at("2026-09-30 16:00", c))
}

@Test func baseDayAndAllDay() throws {
    let c = cal("Asia/Seoul")
    let now = at("2026-09-29 10:00", c)
    let d = try #require(QuickParse.parse("3시 상담", now: now, base: at("2026-10-05 00:00", c), calendar: c))
    #expect(d.startDate == "2026-10-05")
    let a = try #require(QuickParse.parse("금요일 치과", now: now, calendar: c))
    let w = try #require(QuickParse.when(a, calendar: c))
    #expect(w.allDay && w.start == at("2026-10-02 00:00", c) && w.end == w.start)
}

@Test func noDateAndWarnings() throws {
    let c = cal("Asia/Seoul")
    let now = at("2026-09-29 10:00", c)
    let n = try #require(QuickParse.parse("장보기", now: now, calendar: c))
    #expect(n.startDate == nil && n.miss == "no_date" && !n.canDirect && QuickParse.when(n, calendar: c) == nil)
    let r = try #require(QuickParse.parse("매주 월요일 9시 회의", now: now, calendar: c))
    #expect(!r.warnings.isEmpty && !r.canDirect)
}

@Test func skippedHourIsNotGuessed() throws {
    // 뉴욕 2027-03-14 02:30 은 서머타임으로 없는 시각.
    let c = cal("America/New_York")
    let d = try #require(QuickParse.parse("2027년 3월 14일 새벽 2시 30분 알람", now: at("2027-03-01 10:00", c), calendar: c))
    #expect(d.startTime == "02:30")
    #expect(QuickParse.when(d, calendar: c) == nil)
}

@Test func nulInTextIsRejected() {
    #expect(QuickParse.parse("치과\u{0}내일") == nil)
}
