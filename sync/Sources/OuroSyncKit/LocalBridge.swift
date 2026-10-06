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

    /// 보낼 것을 엔진 상태에 다시 넣고 **지금** 보낸다(엔진이 스스로 고르는 때를 기다리지 않는다).
    /// Siri 처럼 앱이 잠깐만 깨어 있을 때 쓴다(개발 14). 저장할 때 `onPending` 이 띄운 알림보다 먼저 와도 되게 직접 다시 알린다.
    @MainActor
    public func push() async {
        let (s, d) = store.pendingRefs()
        guard !s.isEmpty || !d.isEmpty else { return }
        var m = Inbound(op: "pending")
        m.save = s
        m.delete = d
        await core?.receive(m)
        await core?.receive(Inbound(op: "send"))
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
            do {
                try store.apply(msg.records ?? [], deleted: msg.deleted ?? [])
                await core?.receive(Inbound(op: "ack", id: msg.id))
            } catch {
                // 디스크에 못 적었으면 ack 하지 않는다 — 엔진이 «여기까지 받음» 을 저장하지 못하게 멈춰 두고,
                // 다시 켜면 같은 변경을 다시 받는다(맥이 헬퍼를 내리는 것과 같은 효과, 코덱스 개발 11 P0).
                store.sync.error = error.localizedDescription
            }
        case "sent":
            store.sent(saved: msg.saved ?? [], removed: msg.removed ?? [], failed: msg.failed ?? [])
        case "synced":
            // 계정이 없어도 엔진은 «받기 끝» 을 알린다 — 그걸 «맞췄다» 로 보이지 않는다.
            guard store.sync.account == "available" else { return }
            store.sync.lastSync = Date()
            store.sync.error = nil
        case "account":
            switch msg.account {
            case "signIn":
                store.sync.account = "available"
                store.sync.error = nil
                // 로그인 전에 만든 것 — 엔진의 보낼 목록은 로그인 때 비워졌다(코덱스 개발 15).
                let (s, d) = store.pendingRefs()
                store.onPending?(s, d)
            case "signOut", "switchAccounts":
                // 폰의 저장소는 그 계정의 거울 — 비우고, 새 계정이면 엔진이 처음부터 받는다(엔진 상태 파일은 헬퍼 코어가 지웠다).
                store.forgetAccount()
                store.sync.lastSync = nil
                store.sync.error = msg.account == "signOut" ? "iCloud 에서 로그아웃돼 일정을 비웠어요" : "iCloud 계정이 바뀌어 새로 받아요"
            default:
                break
            }
        case "zone_gone":
            if msg.reason == "encryptedDataReset" {
                store.reuploadAll()
            } else {
                store.forgetAccount()
                store.sync.error = "iCloud 에서 Ouro 데이터가 지워졌어요"
            }
        case "error":
            // 계정이 문제면 엔진의 영어 오류 대신 계정 이야기를 한다.
            store.sync.error = Self.accountProblem(store.sync.account) ?? "동기화 오류 — \(msg.message ?? "")"
        default:
            break
        }
    }
}
