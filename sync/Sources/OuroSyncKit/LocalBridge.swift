// 프로세스 안 연결 — iOS 앱은 헬퍼 없이 같은 `SyncCore` 를 직접 쓴다. 맥에선 표준입출력 너머의 러스트가 하던 일
// (need 에 내용 주기·fetched 적고 ack·sent 반영)을 여기서 `RecordStore` 가 한다. 메시지 모양이 같아 엔진 코드는 하나다.

import Foundation

public final class LocalBridge: Outlet, @unchecked Sendable {
    let store: RecordStore
    let stateDir: URL
    private var core: SyncCore?

    @MainActor
    public init(store: RecordStore, containerID: String = "iCloud.com.dinggi5.ouro", stateDir: URL) {
        self.store = store
        self.stateDir = stateDir
        let core = SyncCore(containerID: containerID, stateDir: stateDir, out: self)
        self.core = core
        store.onPending = { [weak self] saves, deletes in
            guard let self else { return }
            var m = Inbound(op: "pending")
            m.save = saves
            m.delete = deletes
            Task { await self.core?.receive(m) }
        }
    }

    /// 엔진을 켜고, 지난번에 못 보낸 것을 다시 알린다.
    @MainActor
    public func start() async {
        try? FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true)
        await core?.start()
        let (s, d) = store.pendingRefs()
        var m = Inbound(op: "pending")
        m.save = s
        m.delete = d
        await core?.receive(m)
    }

    public func fetch() async {
        await core?.receive(Inbound(op: "fetch"))
    }

    public func send(_ msg: Outbound) {
        Task { @MainActor in await self.handle(msg) }
    }

    static func accountProblem(_ a: String?) -> String? {
        switch a {
        case "available": nil
        case "noAccount": "iCloud 에 로그인돼 있지 않아요 — 설정에서 로그인하면 맥과 맞춰요"
        case "restricted": "이 기기에서 iCloud 를 쓸 수 없게 막혀 있어요"
        default: "iCloud 계정을 확인하지 못했어요 — 설정에서 로그인돼 있는지 봐 주세요"
        }
    }

    @MainActor
    func handle(_ msg: Outbound) async {
        switch msg.ev {
        case "ready":
            store.sync.account = msg.account
            store.sync.error = Self.accountProblem(msg.account)
        case "need":
            let (records, missing) = store.provide(msg.names ?? [])
            var m = Inbound(op: "records", id: msg.id)
            m.records = records
            m.missing = missing
            await core?.receive(m)
        case "fetched":
            store.apply(msg.records ?? [], deleted: msg.deleted ?? [])
            await core?.receive(Inbound(op: "ack", id: msg.id))
        case "sent":
            store.sent(saved: msg.saved ?? [], removed: msg.removed ?? [], failed: msg.failed ?? [])
        case "synced":
            // 계정이 없어도 엔진은 «받기 끝» 을 알린다 — 그걸 «맞췄다» 로 보이지 않는다.
            guard store.sync.account == "available" else { return }
            store.sync.lastSync = Date()
            store.sync.error = nil
        case "account":
            if msg.account == "signOut" || msg.account == "switchAccounts" {
                store.sync.error = "iCloud 계정이 바뀌었어요"
            }
        case "error":
            // 계정이 문제면 엔진의 영어 오류 대신 계정 이야기를 한다.
            store.sync.error = Self.accountProblem(store.sync.account) ?? "동기화 오류 — \(msg.message ?? "")"
        default:
            break
        }
    }
}
