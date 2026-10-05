// 앱 ↔ 헬퍼 한 줄짜리 JSON 메시지(표준입력·표준출력, 줄마다 하나).
//
// 앱 → 헬퍼는 `op`, 헬퍼 → 앱은 `ev`. 답을 기다리는 쌍(need→records, fetched→ack)은 `id` 로 짝을 맞춘다.
// 헬퍼는 DB 를 모른다 — 레코드 내용은 매번 앱에 묻고, 받은 변경은 앱이 «적었다(ack)» 고 해야 다음으로 간다.
// 그래서 CKSyncEngine 의 상태(어디까지 받았나)가 앱 DB 보다 앞서 저장되는 일이 없다: 엔진은 이벤트를 하나씩 순서대로 주고,
// `fetched` 의 ack 를 받기 전엔 그 이벤트 처리를 끝내지 않으므로, 그 뒤에 오는 상태 갱신(stateUpdate)도 기다린다.

import Foundation

public struct RecordRef: Codable, Equatable, Sendable {
    public var type: String
    public var name: String
    public init(type: String, name: String) {
        self.type = type
        self.name = name
    }
}

public struct Saved: Codable, Equatable, Sendable {
    public var name: String
    public var system: String
}

public struct Failed: Codable, Equatable, Sendable {
    public var name: String
    /// CKError 코드 이름(`serverRecordChanged`·`unknownItem`·`zoneNotFound`·…) — 앱이 갈래를 고른다.
    public var code: String
    public var reason: String
}

/// 앱 → 헬퍼.
public struct Inbound: Codable, Sendable {
    public var op: String
    public var id: Int?
    public var save: [RecordRef]?
    public var delete: [RecordRef]?
    public var records: [WireRecord]?
    public var missing: [String]?

    public init(op: String, id: Int? = nil) {
        self.op = op
        self.id = id
    }
}

/// 헬퍼 → 앱.
public struct Outbound: Codable, Sendable {
    public var ev: String
    public var id: Int?
    public var account: String?
    public var names: [String]?
    public var records: [WireRecord]?
    public var deleted: [RecordRef]?
    public var saved: [Saved]?
    public var removed: [String]?
    public var failed: [Failed]?
    public var reason: String?
    public var message: String?

    public init(ev: String) { self.ev = ev }
}
