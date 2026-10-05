import Foundation
import Testing
@testable import OuroSyncKit

@MainActor
func twoStores() -> (RecordStore, RecordStore) { (RecordStore(url: nil), RecordStore(url: nil)) }

/// A 의 보낼 것을 B 가 받은 것처럼 — 헬퍼 대신.
@MainActor
func ship(_ a: RecordStore, to b: RecordStore) {
    let (saves, deletes) = a.pendingRefs()
    let (recs, _) = a.provide(saves.map(\.name))
    try! b.apply(recs, deleted: deletes)
    a.sent(saved: recs.map { Saved(name: $0.name, system: "sys") }, removed: deletes.map(\.name), failed: [])
}

@Test @MainActor func eventRoundTripsAndValidates() throws {
    let (a, b) = twoStores()
    var d = EventDraft()
    #expect(throws: OuroError("제목을 적어 주세요")) { try a.createEvent(d) }
    d.title = "  치과  "
    d.start = Date(ms: 1_800_000_000_000)
    d.end = Date(ms: 1_800_003_600_000)
    let id = try a.createEvent(d)
    let rec = a.records[id]!
    #expect(rec.secret["title"] == .string("치과"))
    #expect(rec.fields["title"] == nil)
    #expect(rec.fields["kind"] == .string("event"))
    #expect(rec.secret["prompt"] == .null, "일정이면 부탁 칸은 비운다(맥과 같은 모양)")
    ship(a, to: b)
    #expect(b.events(on: d.start).map(\.title) == ["치과"])
    #expect(a.pendingCount == 0)
}

@Test @MainActor func allDayIsLocalDatesWithExclusiveEnd() throws {
    let s = RecordStore(url: nil)
    var d = EventDraft()
    d.title = "여행"
    d.allDay = true
    let day = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 9))!
    d.start = day
    d.end = Calendar.current.date(byAdding: .day, value: 2, to: day)!
    let id = try s.createEvent(d)
    #expect(s.bag(id)?.str("start_date") == "2026-10-09")
    #expect(s.bag(id)?.str("end_date") == "2026-10-12", "끝은 배타 — 사흘짜리")
    #expect(s.events(on: Calendar.current.date(byAdding: .day, value: 2, to: day)!).count == 1)
    #expect(s.events(on: Calendar.current.date(byAdding: .day, value: 3, to: day)!).isEmpty)
}

@Test @MainActor func newerLocalEditWinsAndIsResent() throws {
    let (a, b) = twoStores()
    var d = EventDraft()
    d.title = "원래"
    let id = try a.createEvent(d)
    ship(a, to: b)
    let old = b.records[id]!
    Clock.now = { Date().addingTimeInterval(5) }
    defer { Clock.now = { Date() } }
    d.title = "새것"
    try a.updateEvent(id, d)
    let resend = try a.apply([old], deleted: [])
    #expect(a.event(id)?.title == "새것")
    #expect(resend == [id])
}

@Test @MainActor func errandFromPhoneIsApprovedAndWaitsForTheMac() throws {
    let s = RecordStore(url: nil)
    var d = ErrandDraft()
    d.prompt = "어제 커밋 정리해 줘\n자세히"
    d.at = Date().addingTimeInterval(3600)
    d.repeatRule = "FREQ=DAILY"
    let id = try s.createErrand(d)
    let e = try #require(s.errand(id))
    #expect(e.title == "어제 커밋 정리해 줘")
    #expect(e.approved)
    #expect(e.run == nil)
    #expect(s.bag(id)?.str("series") == id, "반복이면 첫 회차가 묶음")
    #expect(s.records[id]?.secret["prompt"] == .string("어제 커밋 정리해 줘\n자세히"))
}

@Test @MainActor func answersArriveAndCanBeRead() throws {
    let s = RecordStore(url: nil)
    var d = ErrandDraft()
    d.prompt = "요약"
    let id = try s.createErrand(d)
    let run = WireRecord(
        type: Kind.run, name: "run-1",
        bag: ["item": .string(id), "status": .string("done"), "started_at": .int(1), "finished_at": .int(2), "response": .string("답")],
        system: "s")
    try s.apply([run], deleted: [])
    #expect(s.errand(id)?.run?.response == "답")
    #expect(s.errand(id)?.run?.read == false)
    s.markRead("run-1")
    #expect(s.errand(id)?.run?.read == true)
    #expect(throws: OuroError("이미 돈 부탁은 고칠 수 없어요 — 새 부탁으로 걸어 주세요")) { try s.updateErrand(id, d) }
    // 실행이 있는 부탁은 서버에서 지워져도 남는다.
    try s.apply([], deleted: [RecordRef(type: Kind.item, name: id)])
    #expect(s.errand(id) != nil)
}

@Test @MainActor func proposalBecomesEventOnlyWhenApproved() throws {
    let s = RecordStore(url: nil)
    let now = Clock.ms()
    let p = WireRecord(
        type: Kind.proposal, name: "p1",
        bag: [
            "client": .string("claude-code"), "title": .string("치과"), "notes": .string(""), "all_day": .int(0),
            "start_at": .int(now + 3_600_000), "end_at": .int(now + 7_200_000), "status": .string("pending"), "created_at": .int(now),
        ], system: "s")
    try s.apply([p], deleted: [])
    #expect(s.pendingProposals().count == 1)
    #expect(s.events(on: Date(ms: now + 3_600_000)).isEmpty, "승인 전엔 일정이 아니다")
    let ev = try s.approveProposal("p1")
    #expect(s.event(ev)?.origin == "mcp")
    #expect(s.pendingProposals().isEmpty)
    #expect(throws: OuroError("이미 받은 제안이에요")) { try s.approveProposal("p1") }
}

@Test @MainActor func losingApprovalIsUndone() throws {
    let (a, b) = twoStores()
    let now = Clock.ms()
    let p = WireRecord(
        type: Kind.errandProposal, name: "ep",
        bag: [
            "client": .string("claude"), "prompt": .string("정리"), "start_at": .int(now + 3_600_000), "status": .string("pending"),
            "created_at": .int(now), "late": .string("run"), "target": .string("claude"), "allowed_tools": .string(""), "depth": .int(1),
        ], system: "s")
    try a.apply([p], deleted: [])
    try b.apply([p], deleted: [])
    _ = try a.approveErrandProposal("ep")
    Clock.now = { Date().addingTimeInterval(5) }
    defer { Clock.now = { Date() } }
    let mine = try b.approveErrandProposal("ep")
    ship(a, to: b)
    #expect(b.errand(mine) == nil, "진 승인이 만든 부탁은 휴지통")
    #expect(b.records.values.filter { $0.type == Kind.item && $0.bag.int("deleted_at") == nil }.count == 1)
}

@Test func mergeMatchesTheMacRules() {
    let failed: Bag = ["status": .string("failed"), "finished_at": .int(5)]
    var done = failed
    done["status"] = .string("done")
    done["response"] = .string("답")
    #expect(Merge.merge(Kind.run, local: done, remote: failed)["response"] == .string("답"))
    #expect(Merge.merge(Kind.run, local: failed, remote: done)["response"] == .string("답"))
    let decided: Bag = ["status": .string("approved"), "decided_at": .int(10)]
    #expect(Merge.merge(Kind.proposal, local: decided, remote: ["status": .string("pending")])["status"] == .string("approved"))
}

@Test @MainActor func storeSurvivesRestart() throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("ouro-\(UUID().uuidString)/records.json")
    let s = RecordStore(url: url)
    var d = EventDraft()
    d.title = "남는다"
    let id = try s.createEvent(d)
    let again = RecordStore(url: url)
    #expect(again.event(id)?.title == "남는다")
    #expect(again.pendingCount == 1, "못 보낸 것도 남는다")
}

// ── 맥 ↔ 폰 고정 파일(Fixtures). 맥(Rust `sync.rs` 테스트)이 만든 레코드를 폰이 읽고, 폰이 만든 레코드를 맥이 적는다.
//    다시 만들기: OURO_WRITE_FIXTURE=1 swift test / OURO_WRITE_FIXTURE=1 cargo test fixture

let fixtures = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Fixtures")

@Test @MainActor func readsRecordsMadeByTheMac() throws {
    let data = try Data(contentsOf: fixtures.appendingPathComponent("mac-records.json"))
    let recs = try JSONDecoder().decode([WireRecord].self, from: data)
    let s = RecordStore(url: nil)
    try s.apply(recs, deleted: [])
    let item = try #require(recs.first { $0.type == Kind.item && $0.fields["kind"] == .string("errand") })
    let e = try #require(s.errand(item.name))
    #expect(e.prompt == "어제 커밋 정리해 줘")
    #expect(e.approved)
    #expect(e.run?.response == "정리했어요")
    let ev = try #require(recs.first { $0.type == Kind.item && $0.fields["kind"] == .string("event") })
    #expect(s.event(ev.name)?.title == "치과")
    #expect(s.pendingProposals().first?.event.title == "금요일 재진")
    #expect(s.runner?.name == "맥 A")
}

@Test @MainActor func writesRecordsForTheMac() throws {
    Clock.now = { Date(timeIntervalSince1970: 1_790_000_000) }
    defer { Clock.now = { Date() } }
    let s = RecordStore(url: nil)
    var d = EventDraft()
    d.title = "폰에서 만든 일정"
    d.start = Date(timeIntervalSince1970: 1_790_100_000)
    d.end = d.start.addingTimeInterval(1800)
    d.alertMin = 10
    try s.createEvent(d)
    var r = ErrandDraft()
    r.prompt = "폰에서 건 부탁"
    r.at = Date(timeIntervalSince1970: 1_790_200_000)
    r.target = "codex"
    r.repeatRule = "FREQ=DAILY"
    try s.createErrand(r)
    let (saves, _) = s.pendingRefs()
    let (recs, _) = s.provide(saves.map(\.name).sorted())
    let url = fixtures.appendingPathComponent("ios-records.json")
    if ProcessInfo.processInfo.environment["OURO_WRITE_FIXTURE"] != nil {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try enc.encode(recs).write(to: url)
    }
    // 고정 파일과 칸 이름·모양이 같아야 한다(값의 uid 는 매번 다르다).
    let fixed = try JSONDecoder().decode([WireRecord].self, from: Data(contentsOf: url))
    #expect(Set(fixed.map { Set($0.fields.keys) }) == Set(recs.map { Set($0.fields.keys) }))
    #expect(Set(fixed.map { Set($0.secret.keys) }) == Set(recs.map { Set($0.secret.keys) }))
}

@Test @MainActor func brokenFileIsNeverOverwritten() throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("ouro-\(UUID().uuidString)/records.json")
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    try Data("{망가진".utf8).write(to: url)
    let s = RecordStore(url: url)
    #expect(s.loadError != nil)
    var d = EventDraft()
    d.title = "새 일정"
    #expect(throws: OuroError.self) { try s.createEvent(d) }
    #expect(throws: OuroError.self) { try s.apply([], deleted: []) }
    #expect(try String(contentsOf: url, encoding: .utf8) == "{망가진", "원래 파일은 그대로")
}

@Test @MainActor func editDuringUploadIsAnnouncedAgain() throws {
    let s = RecordStore(url: nil)
    var announced: [String] = []
    s.onPending = { saves, _ in announced += saves.map(\.name) }
    var d = EventDraft()
    d.title = "A"
    let id = try s.createEvent(d)
    _ = s.provide([id])
    d.title = "B"
    try s.updateEvent(id, d)
    announced = []
    s.sent(saved: [Saved(name: id, system: "x")], removed: [], failed: [])
    #expect(s.pendingCount == 1)
    #expect(announced == [id], "엔진이 치운 이름을 다시 알린다")
}

@Test @MainActor func weekdaySeriesStartsOnAWeekday() throws {
    let s = RecordStore(url: nil)
    var r = ErrandDraft()
    r.prompt = "평일 브리핑"
    r.repeatRule = "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"
    // 2026-10-10 은 토요일.
    r.at = Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 10, hour: 9))!
    let id = try s.createErrand(r)
    let at = try #require(s.errand(id)).at
    #expect(Calendar.current.component(.weekday, from: at) == 2, "월요일")
    #expect(Calendar.current.component(.hour, from: at) == 9)
}

@Test @MainActor func accountChangeEmptiesTheMirror() throws {
    let s = RecordStore(url: nil)
    var d = EventDraft()
    d.title = "옛 계정"
    try s.createEvent(d)
    s.forgetAccount()
    #expect(s.records.isEmpty)
    #expect(s.pendingCount == 0)
}

/// 시트를 연 사이 맥에서 고친 게 오면 옛 초안이 덮지 않는다(코덱스 개발 13 P1 — 맥 `update_event_if` 와 같다).
@Test @MainActor func staleSheetDoesNotOverwrite() throws {
    let (phone, mac) = twoStores()
    var d = EventDraft()
    d.title = "치과"
    let id = try phone.createEvent(d)
    ship(phone, to: mac)
    let seen = try #require(phone.event(id)?.updatedAt)
    // 폰 시트가 열려 있는 사이 맥이 메모·숨기기를 고쳤다.
    Clock.now = { Date().addingTimeInterval(5) }
    defer { Clock.now = { Date() } }
    var m = d
    m.notes = "맥에서 적은 메모"
    m.isPrivate = true
    try mac.updateEvent(id, m)
    ship(mac, to: phone)
    var stale = d
    stale.title = "치과 예약"
    #expect(throws: OuroError("다른 곳에서 먼저 고친 일정이에요 — 닫고 다시 열어 주세요")) { try phone.updateEvent(id, stale, seen: seen) }
    #expect(phone.event(id)?.notes == "맥에서 적은 메모")
    #expect(phone.event(id)?.isPrivate == true)
    // 새로 열면(새 판) 고칠 수 있고, 판은 반드시 올라간다.
    let fresh = try #require(phone.event(id)?.updatedAt)
    m.title = "치과 예약"
    try phone.updateEvent(id, m, seen: fresh)
    #expect(try #require(phone.event(id)?.updatedAt) > fresh)

    var e = ErrandDraft()
    e.prompt = "요약해 줘"
    e.at = Date().addingTimeInterval(3600)
    let r = try phone.createErrand(e)
    let rs = try #require(phone.errand(r)?.updatedAt)
    try phone.updateErrand(r, e, seen: rs)
    #expect(throws: OuroError("다른 곳에서 먼저 고친 부탁이에요 — 닫고 다시 열어 주세요")) { try phone.updateErrand(r, e, seen: rs) }
}

/// 부탁 제안은 도구 권한까지 보여 주고, 승인하면 그 권한이 그대로 부탁에 실린다.
@Test @MainActor func errandProposalCarriesItsTools() throws {
    let s = RecordStore(url: nil)
    let now = Clock.ms()
    let p = WireRecord(
        type: Kind.errandProposal, name: "ep1",
        bag: [
            "client": .string("claude-code"), "prompt": .string("PR 리뷰해 줘"), "start_at": .int(now + 3_600_000),
            "target": .string("claude"), "late": .string("run"), "allowed_tools": .string("Read,Grep"),
            "status": .string("pending"), "created_at": .int(now),
        ], system: "s")
    try s.apply([p], deleted: [])
    #expect(s.pendingErrandProposals().first?.allowedTools == "Read,Grep")
    let name = try s.approveErrandProposal("ep1")
    #expect(s.bag(name)?.str("allowed_tools") == "Read,Grep")
}

/// 서버 삭제가 와도 못 보낸 고침은 남기고 다시 올린다(코덱스 개발 14 P0 — 맥 `remote_delete_keeps_unsent_local_edit`).
@Test @MainActor func remoteDeleteKeepsUnsentEdit() throws {
    let (a, b) = twoStores()
    var d = EventDraft()
    d.title = "치과"
    let id = try a.createEvent(d)
    ship(a, to: b)
    d.title = "치과 — 새 메모"
    try b.updateEvent(id, d)
    let resend = try b.apply([], deleted: [RecordRef(type: Kind.item, name: id)])
    #expect(b.event(id)?.title == "치과 — 새 메모")
    #expect(resend == [id])
    // 보낼 게 없는 쪽(a)은 그대로 지운다.
    try a.apply([], deleted: [RecordRef(type: Kind.item, name: id)])
    #expect(a.event(id) == nil)
}
