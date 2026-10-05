import CloudKit
import Foundation
import Testing
@testable import OuroSyncKit

@Test func valueDecodesIntsStringsNullsAndBools() throws {
    let json = #"{"a":1,"b":"x","c":null,"d":true}"#
    let v = try JSONDecoder().decode([String: Value].self, from: Data(json.utf8))
    #expect(v["a"] == .int(1))
    #expect(v["b"] == .string("x"))
    #expect(v["c"] == .null)
    #expect(v["d"] == .int(1))
}

@Test func recordRoundTripKeepsSecretsApart() {
    let w = WireRecord(
        type: "Item", name: "11111111-2222-4333-8444-555555555555",
        fields: ["kind": .string("event"), "start_at": .int(1_700_000_000_000), "deleted_at": .null],
        secret: ["title": .string("치과")], system: nil)
    let r = Wire.ckRecord(w)
    #expect(r.recordID.zoneID == Wire.zoneID)
    #expect(r["title"] == nil, "비밀 칸은 그냥 칸에 없어야 한다")
    #expect(r.encryptedValues["title"] as? String == "치과")
    let back = Wire.wire(r)
    #expect(back.fields["kind"] == .string("event"))
    #expect(back.fields["start_at"] == .int(1_700_000_000_000))
    #expect(back.fields["deleted_at"] == nil, "null 은 칸을 비운다")
    #expect(back.secret["title"] == .string("치과"))
    #expect(back.system != nil)
}

@Test func systemFieldsRebaseAndRejectMismatch() {
    let w = WireRecord(type: "Run", name: "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee", fields: [:], secret: [:], system: nil)
    let sys = Wire.wire(Wire.ckRecord(w)).system
    var again = w
    again.system = sys
    again.fields["status"] = .string("done")
    let r = Wire.ckRecord(again)
    #expect(r["status"] as? String == "done")
    // 다른 이름의 시스템 칸은 버린다.
    var other = w
    other.name = "ffffffff-bbbb-4ccc-8ddd-eeeeeeeeeeee"
    other.system = sys
    #expect(Wire.ckRecord(other).recordID.recordName == other.name)
    // 망가진 값도.
    other.system = "!!!"
    #expect(Wire.ckRecord(other).recordID.recordName == other.name)
}

@Test func inboundParses() throws {
    let json = #"{"op":"pending","save":[{"type":"Item","name":"u1"}],"delete":[]}"#
    let m = try JSONDecoder().decode(Inbound.self, from: Data(json.utf8))
    #expect(m.op == "pending")
    #expect(m.save == [RecordRef(type: "Item", name: "u1")])
}
