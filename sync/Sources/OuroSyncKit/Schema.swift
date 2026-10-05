// 레코드 종류와 칸 목록 — 정본은 맥 앱의 `src-tauri/src/sync.rs`(Spec·merge). 여기는 그 거울이다.
// 칸을 바꾸면 **두 곳을 같이** 고친다(SchemaTests 가 목록을 고정해 둔다).

import Foundation

public enum Kind {
    public static let item = "Item"
    public static let run = "Run"
    public static let proposal = "Proposal"
    public static let errandProposal = "ErrandProposal"
    public static let config = "Config"
    public static let runnerName = "runner"
}

public enum Fields {
    public static let itemPlain = [
        "kind", "all_day", "start_at", "end_at", "start_date", "end_date", "tz", "rrule", "alert_min", "private", "origin",
        "created_at", "updated_at", "deleted_at",
    ]
    public static let itemSecret = ["title", "notes"]
    public static let errandPlain = ["target", "allowed_tools", "approved_at", "depth", "late", "carry", "wall_time"]
    public static let errandSecret = ["prompt", "workdir", "resume_session"]
    public static let errandRefs = ["parent_run", "resume_run", "series", "folder"]
    public static let runPlain = ["started_at", "finished_at", "exit_code", "status", "late_ms", "read_at", "device", "item"]
    public static let runSecret = ["sent_text", "response", "stderr", "session_id", "resumed_session"]
    public static let proposalPlain = [
        "all_day", "start_at", "end_at", "start_date", "end_date", "alert_min", "status", "created_at", "decided_at", "event",
    ]
    public static let proposalSecret = ["client", "title", "notes"]
    public static let errandProposalPlain = [
        "start_at", "allowed_tools", "late", "depth", "status", "created_at", "decided_at", "target", "parent_run", "errand",
    ]
    public static let errandProposalSecret = ["client", "prompt"]
    public static let configPlain = ["device", "name", "at"]

    public static func secret(_ type: String) -> Set<String> {
        switch type {
        case Kind.item: Set(itemSecret + errandSecret)
        case Kind.run: Set(runSecret)
        case Kind.proposal: Set(proposalSecret)
        case Kind.errandProposal: Set(errandProposalSecret)
        default: []
        }
    }

    public static func all(_ type: String) -> [String] {
        switch type {
        case Kind.item: itemPlain + itemSecret + errandPlain + errandSecret + errandRefs
        case Kind.run: runPlain + runSecret
        case Kind.proposal: proposalPlain + proposalSecret
        case Kind.errandProposal: errandProposalPlain + errandProposalSecret
        case Kind.config: configPlain
        default: []
        }
    }
}

/// 칸 지도(그냥 칸 + 비밀 칸을 합친 것). 없는 칸 = null.
public typealias Bag = [String: Value]

extension Bag {
    public func int(_ k: String) -> Int64? {
        if case .int(let i) = self[k] { return i }
        return nil
    }
    public func str(_ k: String) -> String? {
        if case .string(let s) = self[k] { return s }
        return nil
    }
    public func bool(_ k: String) -> Bool { (int(k) ?? 0) != 0 }
}

extension WireRecord {
    public var bag: Bag { fields.merging(secret) { a, _ in a } }

    /// 지도를 종류의 비밀 목록대로 나눠 레코드로.
    public init(type: String, name: String, bag: Bag, system: String?) {
        let hidden = Fields.secret(type)
        var f: [String: Value] = [:]
        var s: [String: Value] = [:]
        for (k, v) in bag {
            if hidden.contains(k) { s[k] = v } else { f[k] = v }
        }
        self.init(type: type, name: name, fields: f, secret: s, system: system)
    }
}

public enum Merge {
    /// 같은 값인가 — 없는 칸과 null 은 같다.
    public static func same(_ a: Bag, _ b: Bag, keys: [String]) -> Bool {
        keys.allSatisfy { (a[$0] ?? .null) == (b[$0] ?? .null) }
    }

    /// 둘 다 가진 레코드를 합친다 — `sync.rs` `merge` 와 같은 규칙.
    public static func merge(_ type: String, local: Bag, remote: Bag) -> Bag {
        switch type {
        case Kind.item:
            return (local.int("updated_at") ?? 0) > (remote.int("updated_at") ?? 0) ? local : remote
        case Kind.config:
            return (local.int("at") ?? 0) > (remote.int("at") ?? 0) ? local : remote
        case Kind.run:
            // 도는 중 < 끝남 < 답 있는 끝남. 그 밖엔 서버 판. 읽음은 어느 쪽이든 — 이른 시각으로.
            func rank(_ m: Bag) -> Int { (m.int("finished_at") != nil ? 1 : 0) + (m.str("response") != nil ? 1 : 0) }
            var m = rank(local) > rank(remote) ? local : remote
            let reads = [local.int("read_at"), remote.int("read_at")].compactMap { $0 }
            m["read_at"] = reads.min().map(Value.int) ?? .null
            return m
        default:
            // 제안: 결정은 되돌아가지 않는다. 둘 다 결정했는데 다르면 먼저 결정한 쪽.
            let ls = local.str("status") ?? "pending"
            let rs = remote.str("status") ?? "pending"
            if ls != "pending" && rs == "pending" { return local }
            if ls != "pending" && rs != "pending" && !same(local, remote, keys: ["status", "decided_at"]) {
                if (local.int("decided_at") ?? .max) < (remote.int("decided_at") ?? .max) { return local }
            }
            return remote
        }
    }
}
