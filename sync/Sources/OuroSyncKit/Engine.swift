// CKSyncEngine 을 감싼 동기화 코어. 개인 DB 의 사용자 지정 존 «Ouro» 하나에 레코드를 둔다.
//
// 흐름(애플 예제 sample-cloudkit-sync-engine 과 같은 뼈대):
//   · 보낼 것: 앱이 `pending` 으로 이름을 알려 주면 엔진 상태에 «보낼 변경» 으로 넣는다. 엔진이 보낼 때가 되면
//     `nextRecordZoneChangeBatch` 에서 앱에 내용을 묻고(need → records) CKRecord 로 만들어 넘긴다.
//   · 받은 것: `fetchedRecordZoneChanges` 를 앱에 그대로 넘기고 ack 를 기다린다.
//   · 엔진 상태(받은 지점·보낼 목록)는 `stateUpdate` 때마다 파일로 — 다음에 켤 때 이어서.
// 헬퍼가 판단하는 건 «CloudKit 의 사정» 뿐이다(존이 없으면 만들기). 무엇이 이기나(충돌)는 앱이 정한다.

import CloudKit
import Foundation

public protocol Outlet: Sendable {
    func send(_ msg: Outbound)
}

public actor SyncCore: CKSyncEngineDelegate {
    let container: CKContainer
    let stateURL: URL
    let out: any Outlet
    var engine: CKSyncEngine?
    var nextID = 1
    var waiting: [Int: CheckedContinuation<Inbound, Never>] = [:]

    public init(containerID: String, stateDir: URL, out: any Outlet) {
        container = CKContainer(identifier: containerID)
        stateURL = stateDir.appendingPathComponent("engine.json")
        self.out = out
    }

    public func start() async {
        let status: String
        do {
            status = Self.accountName(try await container.accountStatus())
        } catch {
            status = "error"
        }
        var ready = Outbound(ev: "ready")
        ready.account = status
        out.send(ready)

        let saved = try? JSONDecoder().decode(
            CKSyncEngine.State.Serialization.self, from: Data(contentsOf: stateURL))
        let config = CKSyncEngine.Configuration(
            database: container.privateCloudDatabase, stateSerialization: saved, delegate: self)
        let e = CKSyncEngine(config)
        engine = e
        if saved == nil {
            // 처음: 존부터. 이미 있으면 서버가 그대로 둔다.
            e.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: Wire.zoneID))])
        }
    }

    static func accountName(_ s: CKAccountStatus) -> String {
        switch s {
        case .available: "available"
        case .noAccount: "noAccount"
        case .restricted: "restricted"
        case .temporarilyUnavailable: "temporarilyUnavailable"
        case .couldNotDetermine: "couldNotDetermine"
        @unknown default: "unknown"
        }
    }

    // MARK: 앱에서 온 메시지

    public func receive(_ m: Inbound) async {
        switch m.op {
        case "pending":
            guard let e = engine else { return }
            let saves = (m.save ?? []).map { Wire.recordID($0.name) }
            let deletes = (m.delete ?? []).map { Wire.recordID($0.name) }
            // 같은 이름의 반대 변경은 걷어 낸다 — 저장과 삭제가 한 묶음에 같이 나가지 않게.
            e.state.remove(pendingRecordZoneChanges: saves.map { .deleteRecord($0) } + deletes.map { .saveRecord($0) })
            e.state.add(pendingRecordZoneChanges: saves.map { .saveRecord($0) } + deletes.map { .deleteRecord($0) })
        case "records", "ack":
            if let id = m.id, let c = waiting.removeValue(forKey: id) {
                c.resume(returning: m)
            }
        case "fetch":
            // 존 하나뿐이라 존을 직접 지정한다 — 데이터베이스 단계(«어느 존이 바뀌었나»)를 건너뛴다.
            await run("fetch") {
                try await $0.fetchChanges(.init(scope: .zoneIDs([Wire.zoneID])))
            }
        case "send":
            await run("send") { try await $0.sendChanges() }
        default:
            log("모르는 op: \(m.op)")
        }
    }

    func run(_ what: String, _ f: (CKSyncEngine) async throws -> Void) async {
        guard let e = engine else { return }
        do {
            try await f(e)
        } catch {
            var m = Outbound(ev: "error")
            m.message = "\(what): \(error.localizedDescription)"
            out.send(m)
        }
    }

    /// 앱에 묻고 답을 기다린다. 앱이 사라지면(표준입력 끝) 헬퍼 프로세스가 통째로 끝나므로 영영 매달리지 않는다.
    func ask(_ msg: Outbound) async -> Inbound {
        let id = nextID
        nextID += 1
        var m = msg
        m.id = id
        return await withCheckedContinuation { c in
            waiting[id] = c
            out.send(m)
        }
    }

    func log(_ s: String) {
        var m = Outbound(ev: "log")
        m.message = s
        out.send(m)
    }

    // MARK: CKSyncEngineDelegate

    public func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let u):
            do {
                let data = try JSONEncoder().encode(u.stateSerialization)
                try data.write(to: stateURL, options: [.atomic])
            } catch {
                log("상태 저장 실패: \(error.localizedDescription)")
            }

        case .accountChange(let a):
            var m = Outbound(ev: "account")
            switch a.changeType {
            case .signIn: m.account = "signIn"
            case .signOut: m.account = "signOut"
            case .switchAccounts: m.account = "switchAccounts"
            @unknown default: m.account = "unknown"
            }
            if m.account != "signIn" {
                // 다른 계정의 받은 지점으로 이어 가면 안 된다.
                try? FileManager.default.removeItem(at: stateURL)
            }
            out.send(m)

        case .fetchedDatabaseChanges(let f):
            for d in f.deletions where d.zoneID == Wire.zoneID {
                var m = Outbound(ev: "zone_gone")
                switch d.reason {
                case .deleted: m.reason = "deleted"
                case .purged: m.reason = "purged"
                case .encryptedDataReset: m.reason = "encryptedDataReset"
                @unknown default: m.reason = "unknown"
                }
                if d.reason == .encryptedDataReset {
                    // 암호 키가 초기화됨 — 존을 다시 만들고 앱이 전부 다시 올린다(애플 문서의 권고).
                    syncEngine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: Wire.zoneID))])
                }
                out.send(m)
            }

        case .fetchedRecordZoneChanges(let f):
            let records = f.modifications.map { Wire.wire($0.record) }
            let deleted = f.deletions.map { RecordRef(type: $0.recordType, name: $0.recordID.recordName) }
            if records.isEmpty && deleted.isEmpty { return }
            var m = Outbound(ev: "fetched")
            m.records = records
            m.deleted = deleted
            _ = await ask(m)

        case .sentRecordZoneChanges(let s):
            var m = Outbound(ev: "sent")
            m.saved = s.savedRecords.map {
                Saved(name: $0.recordID.recordName, system: Wire.encodeSystem($0).base64EncodedString())
            }
            m.removed = s.deletedRecordIDs.map(\.recordName)
            var failed: [Failed] = []
            var conflicts: [WireRecord] = []
            var zoneMissing: [CKRecord.ID] = []
            for f in s.failedRecordSaves {
                let id = f.record.recordID
                let code = Self.codeName(f.error.code)
                switch f.error.code {
                case .serverRecordChanged:
                    // 서버 판을 «받은 것» 처럼 앱에 넘긴다 — 앱이 합치고, 내 쪽이 이기면 다시 보낼 목록에 넣는다.
                    if let server = f.error.serverRecord {
                        conflicts.append(Wire.wire(server))
                    }
                case .zoneNotFound:
                    zoneMissing.append(id)
                default:
                    break
                }
                failed.append(Failed(name: id.recordName, code: code, reason: f.error.localizedDescription))
            }
            for (id, err) in s.failedRecordDeletes {
                failed.append(Failed(name: id.recordName, code: Self.codeName(err.code), reason: err.localizedDescription))
            }
            if !zoneMissing.isEmpty {
                syncEngine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: Wire.zoneID))])
            }
            if !conflicts.isEmpty {
                var c = Outbound(ev: "fetched")
                c.records = conflicts
                c.deleted = []
                _ = await ask(c)
            }
            m.failed = failed
            out.send(m)

        case .didFetchChanges:
            var m = Outbound(ev: "synced")
            m.reason = "fetch"
            out.send(m)

        case .didSendChanges:
            var m = Outbound(ev: "synced")
            m.reason = "send"
            out.send(m)

        case .sentDatabaseChanges(let s):
            for f in s.failedZoneSaves {
                log("존 만들기 실패: \(f.error.localizedDescription)")
            }

        case .willFetchRecordZoneChanges(let w):
            if ProcessInfo.processInfo.environment["OURO_SYNC_TRACE"] != nil { log("존 받기 시작 \(w.zoneID.zoneName)") }
        case .didFetchRecordZoneChanges(let d):
            if ProcessInfo.processInfo.environment["OURO_SYNC_TRACE"] != nil {
                log("존 받기 끝 \(d.zoneID.zoneName) 오류=\(String(describing: d.error))")
            }
        case .willFetchChanges, .willSendChanges:
            break

        @unknown default:
            break
        }
    }

    public func nextRecordZoneChangeBatch(
        _ context: CKSyncEngine.SendChangesContext, syncEngine: CKSyncEngine
    ) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let pending = syncEngine.state.pendingRecordZoneChanges.filter { context.options.scope.contains($0) }
        if pending.isEmpty { return nil }
        // 한 번에 묻는 양을 자른다 — 엔진도 한 묶음에 넣을 만큼만 쓴다.
        let chunk = Array(pending.prefix(200))
        var names: [String] = []
        for c in chunk {
            if case .saveRecord(let id) = c { names.append(id.recordName) }
        }
        var records: [String: CKRecord] = [:]
        if !names.isEmpty {
            var q = Outbound(ev: "need")
            q.names = names
            let reply = await ask(q)
            for w in reply.records ?? [] {
                records[w.name] = Wire.ckRecord(w)
            }
            let gone = Set(reply.missing ?? [])
            if !gone.isEmpty {
                syncEngine.state.remove(pendingRecordZoneChanges: gone.map { .saveRecord(Wire.recordID($0)) })
            }
        }
        let sendable = chunk.filter {
            if case .saveRecord(let id) = $0 { return records[id.recordName] != nil }
            return true
        }
        if sendable.isEmpty { return nil }
        let snapshot = records
        return await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: sendable) { id in
            snapshot[id.recordName]
        }
    }

    static func codeName(_ c: CKError.Code) -> String {
        switch c {
        case .serverRecordChanged: "serverRecordChanged"
        case .unknownItem: "unknownItem"
        case .zoneNotFound: "zoneNotFound"
        case .networkFailure: "networkFailure"
        case .networkUnavailable: "networkUnavailable"
        case .quotaExceeded: "quotaExceeded"
        case .notAuthenticated: "notAuthenticated"
        case .requestRateLimited: "requestRateLimited"
        case .serviceUnavailable: "serviceUnavailable"
        case .zoneBusy: "zoneBusy"
        case .batchRequestFailed: "batchRequestFailed"
        case .limitExceeded: "limitExceeded"
        default: "code\(c.rawValue)"
        }
    }
}
