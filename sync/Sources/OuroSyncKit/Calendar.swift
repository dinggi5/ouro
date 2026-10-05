// 캘린더 모양 — 레코드를 화면이 쓰는 값으로, 화면의 입력을 레코드로. 검사 규칙은 맥(`store.rs` validate · `errands.rs` validate)과 같다.

import Foundation

public enum Rules {
    public static let titleMax = 200
    public static let notesMax = 10_000
    public static let promptMax = 4_000
    public static let titleChars = 40
    /// 고를 수 있는 알림(분 전). 종일은 «그날 9시» 에서 뺀다 — 0 = 당일 9시, 1440 = 전날 9시.
    public static let alertChoices: [Int64] = [0, 5, 10, 15, 30, 60, 120, 1440]
    public static let repeatChoices = ["", "FREQ=DAILY", "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "FREQ=WEEKLY"]
    public static let targets = ["claude", "codex"]
    /// 이만큼 아무도 안 누른 제안은 만료(맥과 같다).
    public static let proposalKeepMs: Int64 = 7 * 24 * 60 * 60 * 1000

    /// 부탁 제목 = 첫 줄 앞 40자.
    public static func title(of prompt: String) -> String {
        let first = prompt.split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        if first.count > titleChars {
            return String(first.prefix(titleChars)).trimmingCharacters(in: .whitespaces) + "…"
        }
        return first
    }
}

public struct OuroError: Error, LocalizedError, Equatable {
    public let message: String
    public init(_ m: String) { message = m }
    public var errorDescription: String? { message }
}

/// 로컬 달력 날짜 `YYYY-MM-DD`.
public enum Day {
    static func formatter() -> DateFormatter {
        let f = DateFormatter()
        f.calendar = Calendar(identifier: .gregorian)
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        f.dateFormat = "yyyy-MM-dd"
        return f
    }
    public static func string(_ d: Date) -> String { formatter().string(from: d) }
    public static func date(_ s: String) -> Date? { s.count == 10 ? formatter().date(from: s) : nil }
}

// MARK: 화면 값

public struct EventView: Identifiable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var notes: String
    public var allDay: Bool
    public var start: Date
    /// 시각 일정 = 끝 시각, 종일 = 끝 날짜(배타)
    public var end: Date
    public var alertMin: Int64?
    public var isPrivate: Bool
    public var origin: String
    /// 판 — 시트가 열 때 본 값. 저장할 때 이게 달라졌으면 다른 기기가 먼저 고친 것(맥 `update_event_if`).
    public var updatedAt: Int64?
}

public struct RunView: Identifiable, Equatable, Sendable {
    public var id: String { name }
    public var name: String
    public var status: String
    public var startedAt: Int64
    public var finishedAt: Int64?
    public var lateMs: Int64
    public var sentText: String
    public var response: String?
    public var stderr: String?
    public var read: Bool

    init(name: String, bag: Bag) {
        self.name = name
        status = bag.str("status") ?? "running"
        startedAt = bag.int("started_at") ?? 0
        finishedAt = bag.int("finished_at")
        lateMs = bag.int("late_ms") ?? 0
        sentText = bag.str("sent_text") ?? ""
        response = bag.str("response")
        stderr = bag.str("stderr")
        read = bag.int("read_at") != nil
    }
}

public struct ErrandView: Identifiable, Equatable, Sendable {
    public var id: String
    public var title: String
    public var prompt: String
    public var at: Date
    public var target: String
    public var late: String
    public var repeatRule: String
    public var approved: Bool
    public var origin: String
    /// 판(`EventView.updatedAt` 와 같다).
    public var updatedAt: Int64?
    /// 가장 최근 실행(없으면 아직 대기).
    public var run: RunView?
}

public struct ProposalView: Identifiable, Equatable, Sendable {
    public var id: String
    public var client: String
    public var event: EventDraft
    public var createdAt: Int64
}

public struct ErrandProposalView: Identifiable, Equatable, Sendable {
    public var id: String
    public var client: String
    public var prompt: String
    public var at: Date
    public var target: String
    /// 놓쳤을 때 — 고치기 시트가 원래 값을 지키게(코덱스 개발 11 P2).
    public var late: String
    /// 승인하면 이 부탁이 쓸 수 있는 도구(맥 `--allowedTools`). 빈 글 = 도구 없이 글만.
    /// 승인 전에 사람이 봐야 한다 — 원문과 함께 «바깥으로 나갈 것» 의 일부다(코덱스 개발 13 P1).
    public var allowedTools: String
    public var createdAt: Int64
}

// MARK: 입력

public struct EventDraft: Equatable, Sendable {
    public var title = ""
    public var notes = ""
    public var allDay = false
    public var start = Date()
    /// 종일이면 마지막 날(포함) — 저장할 때 배타로 하루 더한다.
    public var end = Date()
    public var alertMin: Int64?
    public var isPrivate = false
    public init() {}
}

public struct ErrandDraft: Equatable, Sendable {
    public var prompt = ""
    public var at = Date()
    public var target = "claude"
    public var late = "run"
    public var repeatRule = ""
    public init() {}
}

extension RecordStore {
    // MARK: 목록

    private var cal: Calendar { .current }

    /// 그날 걸치는 일정 — 종일 먼저, 그다음 시작순.
    public func events(on day: Date) -> [EventView] {
        let start = cal.startOfDay(for: day)
        let end = cal.date(byAdding: .day, value: 1, to: start)!
        let ymd = Day.string(start)
        return records.values.compactMap { r -> EventView? in
            guard r.type == Kind.item else { return nil }
            let b = r.bag
            guard b.str("kind") == "event", b.int("deleted_at") == nil, let v = eventView(r.name, b) else { return nil }
            if v.allDay {
                guard let sd = b.str("start_date"), let ed = b.str("end_date"), sd <= ymd, ed > ymd else { return nil }
            } else if !(v.start < end && max(v.end, v.start.addingTimeInterval(0.001)) > start) {
                return nil
            }
            return v
        }
        .sorted { ($0.allDay ? 0 : 1, $0.start, $0.id) < ($1.allDay ? 0 : 1, $1.start, $1.id) }
    }

    func eventView(_ name: String, _ b: Bag) -> EventView? {
        let allDay = b.bool("all_day")
        let start: Date
        let end: Date
        if allDay {
            guard let s = b.str("start_date").flatMap(Day.date), let e = b.str("end_date").flatMap(Day.date) else { return nil }
            (start, end) = (s, e)
        } else {
            guard let s = b.int("start_at"), let e = b.int("end_at") else { return nil }
            (start, end) = (Date(ms: s), Date(ms: e))
        }
        return EventView(
            id: name, title: b.str("title") ?? "", notes: b.str("notes") ?? "", allDay: allDay, start: start, end: end,
            alertMin: b.int("alert_min"), isPrivate: b.bool("private"), origin: b.str("origin") ?? "user", updatedAt: b.int("updated_at"))
    }

    public func event(_ id: String) -> EventView? {
        guard let b = bag(id), b.str("kind") == "event" else { return nil }
        return eventView(id, b)
    }

    /// 그날 예약된 부탁.
    public func errands(on day: Date) -> [ErrandView] {
        let start = cal.startOfDay(for: day)
        let end = cal.date(byAdding: .day, value: 1, to: start)!
        return records.values.compactMap { r -> ErrandView? in
            guard r.type == Kind.item, let v = errand(r.name), v.at >= start, v.at < end else { return nil }
            return v
        }
        .sorted { ($0.at, $0.id) < ($1.at, $1.id) }
    }

    public func errand(_ id: String) -> ErrandView? {
        guard let b = bag(id), b.str("kind") == "errand", b.int("deleted_at") == nil, let at = b.int("start_at") else { return nil }
        return ErrandView(
            id: id, title: b.str("title") ?? "", prompt: b.str("prompt") ?? "", at: Date(ms: at), target: b.str("target") ?? "claude",
            late: b.str("late") ?? "run", repeatRule: b.str("rrule") ?? "", approved: b.int("approved_at") != nil,
            origin: b.str("origin") ?? "user", updatedAt: b.int("updated_at"), run: runs(of: id).first)
    }

    /// 기다리는 제안(만료 전). 오래된 것부터.
    public func pendingProposals(now: Int64 = Clock.ms()) -> [ProposalView] {
        records.values.compactMap { r -> ProposalView? in
            let b = r.bag
            guard r.type == Kind.proposal, b.str("status") == "pending", let c = b.int("created_at"),
                c >= now - Rules.proposalKeepMs
            else { return nil }
            var d = EventDraft()
            d.title = b.str("title") ?? ""
            d.notes = b.str("notes") ?? ""
            d.allDay = b.bool("all_day")
            if d.allDay {
                d.start = b.str("start_date").flatMap(Day.date) ?? Date()
                let e = b.str("end_date").flatMap(Day.date) ?? d.start
                d.end = cal.date(byAdding: .day, value: -1, to: e) ?? d.start
            } else {
                d.start = Date(ms: b.int("start_at") ?? 0)
                d.end = Date(ms: b.int("end_at") ?? 0)
            }
            d.alertMin = b.int("alert_min")
            return ProposalView(id: r.name, client: b.str("client") ?? "AI", event: d, createdAt: c)
        }
        .sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
    }

    public func pendingErrandProposals(now: Int64 = Clock.ms()) -> [ErrandProposalView] {
        records.values.compactMap { r -> ErrandProposalView? in
            let b = r.bag
            guard r.type == Kind.errandProposal, b.str("status") == "pending", let c = b.int("created_at"),
                c >= now - Rules.proposalKeepMs
            else { return nil }
            return ErrandProposalView(
                id: r.name, client: b.str("client") ?? "AI", prompt: b.str("prompt") ?? "",
                at: Date(ms: b.int("start_at") ?? 0), target: b.str("target") ?? "claude", late: b.str("late") ?? "run",
                allowedTools: b.str("allowed_tools") ?? "", createdAt: c)
        }
        .sorted { ($0.createdAt, $0.id) < ($1.createdAt, $1.id) }
    }

    // MARK: 고치기

    /// 일정 칸 검사 → 칸 지도 조각. 맥 `validate` 와 같은 문장.
    func eventFields(_ d: EventDraft) throws -> Bag {
        let title = d.title.trimmingCharacters(in: .whitespacesAndNewlines)
        if title.isEmpty { throw OuroError("제목을 적어 주세요") }
        if title.count > Rules.titleMax { throw OuroError("제목은 \(Rules.titleMax)자까지예요") }
        if d.notes.count > Rules.notesMax { throw OuroError("메모는 \(Rules.notesMax)자까지예요") }
        if let a = d.alertMin, !Rules.alertChoices.contains(a) { throw OuroError("알림 시간이 이상해요") }
        var b: Bag = [
            "title": .string(title), "notes": .string(d.notes), "all_day": .int(d.allDay ? 1 : 0),
            "alert_min": d.alertMin.map(Value.int) ?? .null, "private": .int(d.isPrivate ? 1 : 0),
        ]
        if d.allDay {
            let s = cal.startOfDay(for: d.start)
            let lastDay = cal.startOfDay(for: d.end)
            if lastDay < s { throw OuroError("끝나는 날이 시작보다 앞이에요") }
            let e = cal.date(byAdding: .day, value: 1, to: lastDay)!
            b["start_date"] = .string(Day.string(s))
            b["end_date"] = .string(Day.string(e))
            b["start_at"] = .null
            b["end_at"] = .null
        } else {
            if d.end < d.start { throw OuroError("끝나는 시각이 시작보다 앞이에요") }
            b["start_at"] = .int(d.start.ms)
            b["end_at"] = .int(d.end.ms)
            b["start_date"] = .null
            b["end_date"] = .null
        }
        return b
    }

    static func errandBlank() -> Bag {
        var b: Bag = [:]
        for k in Fields.errandPlain + Fields.errandSecret + Fields.errandRefs { b[k] = .null }
        return b
    }

    private func newItem(kind: String, origin: String, now: Int64) -> Bag {
        var b = Self.errandBlank()
        b.merge([
            "kind": .string(kind), "tz": .string(TimeZone.current.identifier), "rrule": .null, "origin": .string(origin),
            "created_at": .int(now), "updated_at": .int(now), "deleted_at": .null, "notes": .string(""),
            "alert_min": .null, "private": .int(0),
        ]) { _, new in new }
        return b
    }

    @discardableResult
    public func createEvent(_ d: EventDraft, origin: String = "user") throws -> String {
        let now = Clock.ms()
        var b = newItem(kind: "event", origin: origin, now: now)
        b.merge(try eventFields(d)) { _, new in new }
        let name = UUID().uuidString.lowercased()
        try put(Kind.item, name, b)
        return name
    }

    /// `seen` = 시트를 열 때 본 판(`updated_at`). 그 사이 맥이 고친 게 동기화로 왔으면 덮지 않고 거절한다 —
    /// 옛 초안이 맥에서 고친 메모·«AI 에게 숨기기» 를 되돌리지 않게(맥 `update_event_if` 와 같다, 코덱스 개발 13 P1). nil = 확인 안 함.
    public func updateEvent(_ id: String, _ d: EventDraft, seen: Int64? = nil) throws {
        guard var b = bag(id), b.str("kind") == "event", b.int("deleted_at") == nil else { throw OuroError("이미 지워진 일정이에요") }
        if let seen, b.int("updated_at") != seen { throw OuroError("다른 곳에서 먼저 고친 일정이에요 — 닫고 다시 열어 주세요") }
        b.merge(try eventFields(d)) { _, new in new }
        b["updated_at"] = .int(Self.nextRev(b))
        try put(Kind.item, id, b)
    }

    /// 새 판 = 지금, 단 옛 판보다 반드시 크게(맥 `MAX(now, updated_at + 1)`) — 시계가 뒤로 간 기기에서 고쳐도
    /// 판이 같아져 «고친 걸 못 알아보는» 일이 없게.
    static func nextRev(_ b: Bag) -> Int64 {
        max(Clock.ms(), (b.int("updated_at") ?? 0) + 1)
    }

    /// 휴지통으로(일정·부탁 둘 다). 맥이 7일 뒤 비우면 그 삭제가 동기화로 온다.
    public func trash(_ id: String) throws {
        guard var b = bag(id), b.int("deleted_at") == nil else { throw OuroError("이미 지워졌어요") }
        b["deleted_at"] = .int(Clock.ms())
        b["updated_at"] = .int(Self.nextRev(b))
        try put(Kind.item, id, b)
    }

    public func restore(_ id: String) throws {
        guard var b = bag(id), b.int("deleted_at") != nil else { throw OuroError("되살릴 게 없어요") }
        b["deleted_at"] = .null
        b["updated_at"] = .int(Self.nextRev(b))
        try put(Kind.item, id, b)
    }

    func errandFields(_ d: ErrandDraft) throws -> Bag {
        let prompt = d.prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if prompt.isEmpty { throw OuroError("부탁할 말을 적어 주세요") }
        if prompt.count > Rules.promptMax { throw OuroError("부탁은 \(Rules.promptMax)자까지예요") }
        if !["run", "skip"].contains(d.late) { throw OuroError("놓쳤을 때 할 일이 이상해요") }
        if !Rules.targets.contains(d.target) { throw OuroError("부탁 받을 쪽이 이상해요") }
        if !Rules.repeatChoices.contains(d.repeatRule) { throw OuroError("반복이 이상해요") }
        let at = firstAt(d).ms
        return [
            "title": .string(Rules.title(of: prompt)), "prompt": .string(prompt), "start_at": .int(at), "end_at": .int(at),
            "all_day": .int(0), "start_date": .null, "end_date": .null, "target": .string(d.target), "late": .string(d.late),
            "rrule": d.repeatRule.isEmpty ? .null : .string(d.repeatRule),
        ]
    }

    /// 평일 반복인데 주말에 걸면 첫 회차를 다음 월요일 같은 시각으로(맥 `first_at` 과 같다 — 실행 맥은 받은 시각 그대로 돌린다).
    func firstAt(_ d: ErrandDraft) -> Date {
        var at = Date(timeIntervalSince1970: (d.at.timeIntervalSince1970 / 60).rounded(.down) * 60)
        if d.repeatRule == "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" {
            // 규칙이 월~금 고정이라 지역 설정의 주말(금·토 등)이 아니라 토·일로 본다(코덱스 개발 11 2차).
            while [1, 7].contains(Calendar(identifier: .gregorian).component(.weekday, from: at)) {
                at = cal.date(byAdding: .day, value: 1, to: at)!
            }
        }
        return at
    }

    /// 사람이 폰에서 거는 부탁 — 만들 때 승인이 찍힌다(사람이 쓴 글). 돌리는 건 실행 맥.
    @discardableResult
    public func createErrand(_ d: ErrandDraft) throws -> String {
        let now = Clock.ms()
        var b = newItem(kind: "errand", origin: "user", now: now)
        b.merge(try errandFields(d)) { _, new in new }
        let name = UUID().uuidString.lowercased()
        b.merge([
            "allowed_tools": .string(""), "approved_at": .int(now), "depth": .int(0), "carry": .int(0),
            // 반복이면 첫 회차가 곧 묶음이다(맥 `insert_errand` 와 같다).
            "series": d.repeatRule.isEmpty ? .null : .string(name),
        ]) { _, new in new }
        try put(Kind.item, name, b)
        return name
    }

    /// 아직 안 돈 부탁만 고친다(돈 부탁의 글은 «보낸 원문» 과 어긋나면 안 된다 — 맥도 대기 중인 것만 고친다).
    public func updateErrand(_ id: String, _ d: ErrandDraft, seen: Int64? = nil) throws {
        guard var b = bag(id), b.str("kind") == "errand", b.int("deleted_at") == nil else { throw OuroError("이미 지워진 부탁이에요") }
        if let seen, b.int("updated_at") != seen { throw OuroError("다른 곳에서 먼저 고친 부탁이에요 — 닫고 다시 열어 주세요") }
        if !runs(of: id).isEmpty { throw OuroError("이미 돈 부탁은 고칠 수 없어요 — 새 부탁으로 걸어 주세요") }
        // 잇는 대화가 있으면 받는 쪽·반복을 못 바꾼다(맥 `update_errand` 와 같다 — 실행 맥은 받은 값을 다시 검사하지 않는다).
        if b.str("resume_session") != nil || b.str("resume_run") != nil,
            d.target != b.str("target") || !d.repeatRule.isEmpty
        {
            throw OuroError("이어서 하는 부탁은 받는 쪽·반복을 바꿀 수 없어요")
        }
        let oldStart = b.int("start_at")
        b.merge(try errandFields(d)) { _, new in new }
        // 사람이 시각을 옮기면 그 시각이 새 기준 — 서머타임으로 밀렸던 원래 시각은 버린다(맥과 같다).
        if b.int("start_at") != oldStart { b["wall_time"] = .null }
        if d.repeatRule.isEmpty == false, b.str("series") == nil { b["series"] = .string(id) }
        b["updated_at"] = .int(Self.nextRev(b))
        try put(Kind.item, id, b)
    }

    public func markRead(_ run: String) {
        guard var b = bag(run), b.int("read_at") == nil, b.str("status") == "done" else { return }
        b["read_at"] = .int(Clock.ms())
        try? put(Kind.run, run, b)
    }

    // MARK: 제안 — 사람이 누르는 승인만이 일정·부탁을 만든다(CLAUDE.md 불변 규칙)

    private func pendingProposal(_ id: String, _ type: String) throws -> Bag {
        guard let b = bag(id), records[id]?.type == type else { throw OuroError("없는 제안이에요") }
        switch b.str("status") ?? "pending" {
        case "pending":
            if let c = b.int("created_at"), c < Clock.ms() - Rules.proposalKeepMs { throw OuroError("오래돼서 만료된 제안이에요") }
            return b
        case "approved": throw OuroError("이미 받은 제안이에요")
        case "rejected": throw OuroError("이미 거절한 제안이에요")
        default: throw OuroError("오래돼서 만료된 제안이에요")
        }
    }

    /// 일정 제안 승인 — `edited` 가 있으면 사람이 고친 값으로.
    @discardableResult
    public func approveProposal(_ id: String, edited: EventDraft? = nil) throws -> String {
        var p = try pendingProposal(id, Kind.proposal)
        let draft = edited ?? pendingProposals(now: Clock.ms()).first { $0.id == id }?.event ?? EventDraft()
        let now = Clock.ms()
        var b = newItem(kind: "event", origin: "mcp", now: now)
        b.merge(try eventFields(draft)) { _, new in new }
        let event = UUID().uuidString.lowercased()
        p["status"] = .string("approved")
        p["decided_at"] = .int(now)
        p["event"] = .string(event)
        try putAll([(Kind.item, event, b), (Kind.proposal, id, p)])
        return event
    }

    public func rejectProposal(_ id: String) throws {
        let type = records[id]?.type ?? Kind.proposal
        var p = try pendingProposal(id, type)
        p["status"] = .string("rejected")
        p["decided_at"] = .int(Clock.ms())
        try put(type, id, p)
    }

    /// 부탁 제안 승인 — 🔴 AI 가 낸 글이 부탁이 되는 길. 사람이 본(고친) 글이 저장되고 승인이 찍힌다.
    @discardableResult
    public func approveErrandProposal(_ id: String, edited: ErrandDraft? = nil) throws -> String {
        var p = try pendingProposal(id, Kind.errandProposal)
        var d = edited ?? ErrandDraft()
        if edited == nil {
            d.prompt = p.str("prompt") ?? ""
            d.at = Date(ms: p.int("start_at") ?? 0)
            d.target = p.str("target") ?? "claude"
            d.late = p.str("late") ?? "run"
            if d.at.ms < Clock.ms() - 60_000 { throw OuroError("제안한 때가 이미 지났어요 — 고쳐서 새 때를 정해 주세요") }
        }
        let now = Clock.ms()
        var b = newItem(kind: "errand", origin: "mcp", now: now)
        b.merge(try errandFields(d)) { _, new in new }
        let name = UUID().uuidString.lowercased()
        b.merge([
            "allowed_tools": p["allowed_tools"] ?? .string(""), "approved_at": .int(now), "depth": p["depth"] ?? .int(0),
            "parent_run": p["parent_run"] ?? .null, "carry": .int(0),
            "series": d.repeatRule.isEmpty ? .null : .string(name),
        ]) { _, new in new }
        p["status"] = .string("approved")
        p["decided_at"] = .int(now)
        p["errand"] = .string(name)
        try putAll([(Kind.item, name, b), (Kind.errandProposal, id, p)])
        return name
    }
}

extension Date {
    public init(ms: Int64) { self.init(timeIntervalSince1970: Double(ms) / 1000) }
    public var ms: Int64 { Int64((timeIntervalSince1970 * 1000).rounded()) }
}
