// 앱(러스트) ↔ 헬퍼 사이의 레코드 모양과 CKRecord 변환.
//
// 레코드 = «종류 + 이름(uid) + 칸들». 칸은 둘로 나뉜다:
//   · fields — 그냥 칸(시각·상태·숫자). CloudKit 서버가 값을 본다.
//   · secret — `encryptedValues`(종단간 암호화): 제목·메모·부탁 글·답. 애플도 못 읽는다(PLAN §13).
// 값은 문자열·정수·null 뿐이다(불리언은 0/1, 시각은 UTC 밀리초). null = 칸을 비운다(CKRecord 에서 키가 사라진다).
// 어느 칸이 secret 인지는 **앱이 정한다** — 헬퍼는 받은 대로 담을 뿐이라, 종류가 늘어도 헬퍼를 고치지 않는다.

import CloudKit
import Foundation

public enum Value: Codable, Equatable, Sendable {
    case string(String)
    case int(Int64)
    case null

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let i = try? c.decode(Int64.self) {
            self = .int(i)
        } else if let b = try? c.decode(Bool.self) {
            self = .int(b ? 1 : 0)
        } else {
            self = .string(try c.decode(String.self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .string(let s): try c.encode(s)
        case .int(let i): try c.encode(i)
        case .null: try c.encodeNil()
        }
    }

    var ck: (any __CKRecordObjCValue)? {
        switch self {
        case .string(let s): return s as NSString
        case .int(let i): return NSNumber(value: i)
        case .null: return nil
        }
    }

    /// CKRecord 에서 읽은 값. 우리가 쓰지 않는 모양(날짜·에셋 등)은 null 로 — 모르는 걸 지어내지 않는다.
    init(ck: Any?) {
        switch ck {
        case let s as String: self = .string(s)
        case let n as NSNumber: self = .int(n.int64Value)
        default: self = .null
        }
    }
}

public struct WireRecord: Codable, Equatable, Sendable {
    public var type: String
    public var name: String
    public var fields: [String: Value]
    public var secret: [String: Value]
    /// CKRecord 시스템 칸(변경 태그 등)을 `encodeSystemFields` 로 굳힌 것, base64. 앱이 레코드와 같이 DB 에 둔다 —
    /// 다음 저장 때 이걸 바탕으로 해야 서버가 «내가 본 판에서 고친 것» 으로 받는다(없으면 매번 충돌).
    public var system: String?

    public init(type: String, name: String, fields: [String: Value], secret: [String: Value], system: String?) {
        self.type = type
        self.name = name
        self.fields = fields
        self.secret = secret
        self.system = system
    }
}

public enum Wire {
    public static let zoneID = CKRecordZone.ID(zoneName: "Ouro")

    public static func recordID(_ name: String) -> CKRecord.ID {
        CKRecord.ID(recordName: name, zoneID: zoneID)
    }

    /// 앱이 준 레코드를 CKRecord 로. 시스템 칸이 있으면 그 판 위에 칸을 덮는다.
    /// 시스템 칸의 이름·종류가 앱이 말한 것과 다르면(망가진 값) 버리고 새로 만든다 — 엉뚱한 레코드를 덮어쓰지 않게.
    public static func ckRecord(_ w: WireRecord) -> CKRecord {
        var rec: CKRecord?
        if let s = w.system, let data = Data(base64Encoded: s) {
            rec = decodeSystem(data)
            if let r = rec, r.recordID.recordName != w.name || r.recordType != w.type || r.recordID.zoneID != zoneID {
                rec = nil
            }
        }
        let r = rec ?? CKRecord(recordType: w.type, recordID: recordID(w.name))
        for (k, v) in w.fields {
            r[k] = v.ck
        }
        for (k, v) in w.secret {
            r.encryptedValues[k] = v.ck
        }
        return r
    }

    /// 서버에서 온 CKRecord 를 앱에 줄 모양으로.
    public static func wire(_ r: CKRecord) -> WireRecord {
        // `allKeys()` 는 암호화 칸의 이름도 돌려준다(값은 nil) — 그건 secret 쪽에만 둔다.
        let hidden = Set(r.encryptedValues.allKeys())
        var fields: [String: Value] = [:]
        for k in r.allKeys() where !hidden.contains(k) {
            fields[k] = Value(ck: r[k])
        }
        var secret: [String: Value] = [:]
        for k in r.encryptedValues.allKeys() {
            secret[k] = Value(ck: r.encryptedValues[k])
        }
        return WireRecord(
            type: r.recordType, name: r.recordID.recordName, fields: fields, secret: secret,
            system: encodeSystem(r).base64EncodedString())
    }

    public static func encodeSystem(_ r: CKRecord) -> Data {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        r.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData
    }

    public static func decodeSystem(_ data: Data) -> CKRecord? {
        guard let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }
}
