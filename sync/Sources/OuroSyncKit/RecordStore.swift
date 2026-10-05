// iOS 쪽 저장소 — 레코드를 그대로(uid 이름, 칸 지도) 들고 있다. 맥은 SQLite 표로 풀어 두지만, 폰은 실행·MCP 가 없어
// 표로 풀 이유가 없다: 참조도 uid 그대로라 «아직 못 푼 참조» 가 없다.
//
// 맥 앱과 같은 약속:
//   · 고치면 `updated_at` 을 올리고 «보낼 것» 에 넣는다. 받은 것은 `Merge` 규칙으로 합치고, 내 쪽이 이기면 다시 보낸다.
//   · 사람이 만든 부탁은 만들 때 승인(`approved_at`)이 찍힌다. AI 제안은 사람이 «승인» 을 눌러야 부탁·일정이 된다.
//   · 폰은 부탁을 **돌리지 않는다** — 실행 맥이 돌리고 답이 동기화로 온다(PLAN §13).
// 디스크: 한 JSON 파일(원자적 쓰기). 일정 수천 개여도 수 MB — 폰의 캘린더 크기에 충분하다.

import Foundation
import Observation

public struct SyncInfo: Equatable, Sendable {
    public var account: String?
    public var lastSync: Date?
    public var error: String?
    public init() {}
}

@MainActor
@Observable
public final class RecordStore {
    public private(set) var records: [String: WireRecord] = [:]
    /// 보낼 것: 이름 → 고친 차례(보내는 사이 또 고쳤는지 가리려고).
    private(set) var saves: [String: Int] = [:]
    /// 지울 것: 이름 → 종류.
    private(set) var deletes: [String: String] = [:]
    private var version = 0
    private var inflight: [String: Int] = [:]
    public var sync = SyncInfo()
    /// 보낼 것이 바뀌면 부른다(브리지가 헬퍼 엔진에 알린다).
    @ObservationIgnored public var onPending: (([RecordRef], [RecordRef]) -> Void)?
    @ObservationIgnored let url: URL?

    struct Disk: Codable {
        var records: [String: WireRecord]
        var saves: [String: Int]
        var deletes: [String: String]
        var version: Int
    }

    /// 저장 파일이 있는데 못 읽었다 — 이 상태에선 아무것도 쓰지 않는다(빈 저장소로 덮으면 못 보낸 일정이 사라진다, 코덱스 개발 11 P0).
    public private(set) var loadError: String?

    /// `url` 이 nil 이면 메모리에만(테스트).
    public init(url: URL?) {
        self.url = url
        guard let url, FileManager.default.fileExists(atPath: url.path) else { return }
        do {
            let d = try JSONDecoder().decode(Disk.self, from: Data(contentsOf: url))
            records = d.records
            saves = d.saves
            deletes = d.deletes
            version = d.version
        } catch {
            loadError = "저장한 일정을 못 읽었어요 — 앱을 다시 켜 주세요(\(error.localizedDescription))"
            sync.error = loadError
        }
    }

    /// 디스크에 쓴다. 실패하면 던진다 — 받은 변경을 «적었다(ack)» 고 하기 전에 확인해야 한다(코덱스 개발 11 P0).
    func persist() throws {
        if let loadError { throw OuroError(loadError) }
        guard let url else { return }
        let d = Disk(records: records, saves: saves, deletes: deletes, version: version)
        do {
            try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            #if os(iOS)
                // 잠긴 폰에선 못 읽는다 — 일정·답은 남이 볼 게 아니다.
                let options: Data.WritingOptions = [.atomic, .completeFileProtectionUntilFirstUserAuthentication]
            #else
                let options: Data.WritingOptions = [.atomic]
            #endif
            try JSONEncoder().encode(d).write(to: url, options: options)
        } catch {
            sync.error = "기기에 저장하지 못했어요: \(error.localizedDescription)"
            throw OuroError("기기에 저장하지 못했어요 — 저장 공간을 확인해 주세요")
        }
    }

    /// 여러 레코드를 한 번에(제안 승인 = 새 항목 + 제안의 결정). 디스크에 못 쓰면 메모리도 되돌린다 —
    /// 반쯤 쓰인 채 다시 켜면 «승인된 부탁 + 아직 기다리는 제안» 이 남아 두 번 승인될 수 있다(코덱스 개발 11 P1).
    func putAll(_ items: [(String, String, Bag)]) throws {
        let before = (records, saves, deletes, version)
        for (type, name, bag) in items {
            records[name] = WireRecord(type: type, name: name, bag: bag, system: records[name]?.system)
            markSave(name)
        }
        do {
            try persist()
        } catch {
            (records, saves, deletes, version) = before
            throw error
        }
        announce(items.map(\.1))
    }

    /// 지금 보낼 것 전부(켤 때·실패 뒤 다시 알리기).
    public func pendingRefs() -> ([RecordRef], [RecordRef]) {
        let s = saves.keys.compactMap { n in records[n].map { RecordRef(type: $0.type, name: n) } }
        let d = deletes.map { RecordRef(type: $0.value, name: $0.key) }
        return (s, d)
    }

    private func markSave(_ name: String) {
        version += 1
        saves[name] = version
        deletes.removeValue(forKey: name)
    }

    private func announce(_ names: [String]) {
        let s = names.compactMap { n in records[n].map { RecordRef(type: $0.type, name: n) } }
        onPending?(s, [])
    }

    // MARK: 이 기기에서 고치기

    /// 레코드 하나를 이 칸들로 덮는다(시스템 칸은 그대로). 보낼 것에 넣고 알린다.
    func put(_ type: String, _ name: String, _ bag: Bag) throws {
        try putAll([(type, name, bag)])
    }

    // MARK: 동기화(브리지가 부른다)

    /// 헬퍼가 보낼 내용을 묻는다.
    public func provide(_ names: [String]) -> ([WireRecord], [String]) {
        var out: [WireRecord] = []
        var missing: [String] = []
        for n in names {
            if let r = records[n] {
                out.append(r)
                inflight[n] = saves[n] ?? version
            } else {
                missing.append(n)
            }
        }
        return (out, missing)
    }

    /// 받은 묶음을 합친다. 돌려주는 값 = 다시 보낼 이름들(내 쪽이 이긴 것).
    @discardableResult
    public func apply(_ incoming: [WireRecord], deleted: [RecordRef]) throws -> [String] {
        if let loadError { throw OuroError(loadError) }
        let before = (records, saves, deletes, version)
        var resend: [String] = []
        for ref in deleted {
            if ref.type == Kind.item, hasRuns(ref.name), let r = records[ref.name] {
                // 실행 기록이 있는 부탁은 지우지 않고 새로 올린다(바깥으로 나간 원문 보존 — 맥과 같은 규칙).
                records[ref.name] = WireRecord(type: r.type, name: r.name, fields: r.fields, secret: r.secret, system: nil)
                markSave(ref.name)
                resend.append(ref.name)
                continue
            }
            records.removeValue(forKey: ref.name)
            saves.removeValue(forKey: ref.name)
            deletes.removeValue(forKey: ref.name)
        }
        for rec in incoming {
            let keys = Fields.all(rec.type)
            if keys.isEmpty { continue } // 모르는 종류(더 새 버전) — 건드리지 않는다.
            let remote = rec.bag
            let merged: Bag
            if let local = records[rec.name]?.bag {
                merged = Merge.merge(rec.type, local: local, remote: remote)
                undoLosingApproval(rec.type, local: local, merged: merged, resend: &resend)
            } else {
                merged = remote
            }
            records[rec.name] = WireRecord(type: rec.type, name: rec.name, bag: merged, system: rec.system)
            if Merge.same(merged, remote, keys: keys) {
                saves.removeValue(forKey: rec.name)
            } else {
                markSave(rec.name)
                resend.append(rec.name)
            }
        }
        do {
            try persist()
        } catch {
            (records, saves, deletes, version) = before
            throw error
        }
        if !resend.isEmpty { announce(resend) }
        return resend
    }

    /// 헬퍼가 서버 저장 결과를 알린다.
    public func sent(saved: [Saved], removed: [String], failed: [Failed]) {
        var still: [String] = []
        for s in saved {
            if let r = records[s.name] {
                records[s.name] = WireRecord(type: r.type, name: r.name, fields: r.fields, secret: r.secret, system: s.system)
            }
            if let v = inflight.removeValue(forKey: s.name), saves[s.name] == v {
                saves.removeValue(forKey: s.name)
            } else if saves[s.name] != nil {
                // 보내는 사이 또 고쳤다 — 엔진은 이 이름을 «보냄» 으로 치웠으니 다시 알린다(코덱스 개발 11 P1).
                still.append(s.name)
            }
        }
        for n in removed {
            deletes.removeValue(forKey: n)
        }
        var again = false
        for f in failed {
            inflight.removeValue(forKey: f.name)
            switch f.code {
            case "unknownItem":
                // 서버에 없다 — 옛 시스템 칸을 버리고 새로 올린다. 삭제였다면 이미 없는 것.
                if let r = records[f.name] {
                    records[f.name] = WireRecord(type: r.type, name: r.name, fields: r.fields, secret: r.secret, system: nil)
                }
                deletes.removeValue(forKey: f.name)
            case "quotaExceeded":
                sync.error = "iCloud 저장 공간이 꽉 찼어요"
            default:
                break
            }
            again = true
        }
        try? persist()
        if again {
            let (s, d) = pendingRefs()
            onPending?(s, d)
        } else if !still.isEmpty {
            announce(still)
        }
    }

    /// iCloud 계정이 바뀌었거나 서버의 Ouro 데이터가 지워졌다 — 폰의 저장소는 그 계정의 거울이라 비운다
    /// (다른 계정에 옛 계정의 글을 올리지 않게, 코덱스 개발 11 P1). 새 계정으로 로그인하면 엔진이 처음부터 받는다.
    public func forgetAccount() {
        records = [:]
        saves = [:]
        deletes = [:]
        inflight = [:]
        try? persist()
    }

    /// 서버 암호 키가 초기화됨 — 시스템 칸을 버리고 가진 것을 전부 다시 올린다(맥과 같은 처리).
    public func reuploadAll() {
        for (n, r) in records {
            records[n] = WireRecord(type: r.type, name: r.name, fields: r.fields, secret: r.secret, system: nil)
            markSave(n)
        }
        try? persist()
        let (s, d) = pendingRefs()
        onPending?(s, d)
    }

    /// 같은 제안을 두 기기가 따로 결정했고 이 기기의 결정이 졌다 — 진 승인이 만든 항목을 휴지통으로(한 번도 안 돈 것만).
    private func undoLosingApproval(_ type: String, local: Bag, merged: Bag, resend: inout [String]) {
        let field: String
        switch type {
        case Kind.proposal: field = "event"
        case Kind.errandProposal: field = "errand"
        default: return
        }
        guard let mine = local.str(field), merged.str(field) != mine, var item = records[mine]?.bag,
            item.int("deleted_at") == nil, !hasRuns(mine)
        else { return }
        let now = Clock.ms()
        item["deleted_at"] = .int(now)
        item["updated_at"] = .int(now)
        records[mine] = WireRecord(type: Kind.item, name: mine, bag: item, system: records[mine]?.system)
        markSave(mine)
        resend.append(mine)
    }

    // MARK: 읽기

    public func bag(_ name: String) -> Bag? { records[name]?.bag }

    func hasRuns(_ item: String) -> Bool {
        records.values.contains { $0.type == Kind.run && $0.fields["item"] == .string(item) }
    }

    /// 한 부탁의 실행들, 최근 것부터(시작 시각 — 받은 순서가 아니라).
    public func runs(of item: String) -> [RunView] {
        records.values
            .filter { $0.type == Kind.run && $0.fields["item"] == .string(item) }
            .map { RunView(name: $0.name, bag: $0.bag) }
            .sorted { ($0.startedAt, $0.name) > ($1.startedAt, $1.name) }
    }

    /// 지금 보낼 것이 몇 개 남았나(상태 줄).
    public var pendingCount: Int { saves.count + deletes.count }

    public var runner: (name: String, device: String)? {
        guard let b = records[Kind.runnerName]?.bag, let d = b.str("device"), !d.isEmpty else { return nil }
        return (b.str("name") ?? "맥", d)
    }
}

/// 시각 — 테스트가 바꿀 수 있게 한 곳에서.
public enum Clock {
    nonisolated(unsafe) public static var now: () -> Date = { Date() }
    public static func ms() -> Int64 { Int64((now().timeIntervalSince1970 * 1000).rounded()) }
}
