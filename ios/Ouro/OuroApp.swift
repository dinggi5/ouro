// Ouro iOS — 맥의 Ouro 와 iCloud 로 오가는 캘린더(개발 11). 보기·쓰기·부탁 걸기·답 읽기·AI 제안 승인.
// 부탁은 폰이 돌리지 않는다 — «실행 맥» 이 돌리고 답이 동기화로 온다.
// 개발 14: 빠른 추가(입력칸 · 컨트롤 버튼 · Siri·단축어). 인텐트는 앱 프로세스에서 돌아 화면과 같은 `AppModel.shared` 를 쓴다.

import OuroSyncKit
import SwiftUI
import UIKit

final class AppDelegate: NSObject, UIApplicationDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        // 다른 기기가 바꾸면 곧바로 받기(CKSyncEngine 이 푸시를 받아 스스로 가져온다).
        application.registerForRemoteNotifications()
        return true
    }
}

/// 바깥(컨트롤·Siri)이 화면에 부탁하는 것.
enum AppRequest: Equatable {
    /// 입력칸에 포커스.
    case quickAdd
    /// 사람이 보고 저장해야 하는 초안(날짜를 못 찾았거나 경고가 있을 때) — 일정 시트로 연다.
    case sheet(EventDraft)
}

@MainActor
@Observable
final class AppModel {
    /// 하나뿐 — Siri 가 앱을 화면 없이 깨워도 화면과 같은 저장소를 쓴다(저장 파일을 두 객체가 번갈아 덮지 않게).
    static let shared = AppModel()

    @ObservationIgnored let store: RecordStore
    @ObservationIgnored let bridge: LocalBridge
    @ObservationIgnored let demo = ProcessInfo.processInfo.arguments.contains("-demo")
    var request: AppRequest?
    @ObservationIgnored private var started = false

    private init() {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Ouro")
        store = RecordStore(url: demo ? nil : base.appendingPathComponent("records.json"))
        bridge = LocalBridge(store: store, stateDir: base.appendingPathComponent("sync"))
        if demo { Demo.fill(store) }
    }

    /// 동기화를 켠다(한 번만). 화면이 뜰 때와, Siri 가 화면 없이 깨웠을 때 둘 다 부른다.
    func startSync() async {
        guard !started, !demo else { return }
        started = true
        await bridge.start()
    }

    func fetch() async {
        guard started else { return }
        await bridge.fetch()
    }

    /// 방금 넣은 것을 지금 올리고, 다 올라갈 때까지 잠깐 기다린다 — Siri 로 넣고 앱이 곧 잠들면 다음에 켤 때까지 안 올라간다.
    /// 못 기다려도 잃지 않는다: 보낼 것은 디스크에 남아 다음에 켤 때 다시 알린다.
    func pushNow(wait: Duration = .seconds(6)) async {
        guard !demo else { return }
        await startSync()
        Task { await bridge.push() }
        let until = ContinuousClock.now + wait
        while store.pendingCount > 0, ContinuousClock.now < until {
            try? await Task.sleep(for: .milliseconds(200))
        }
    }
}

@main
struct OuroApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @Environment(\.scenePhase) private var phase
    @State private var model = AppModel.shared

    var body: some Scene {
        WindowGroup {
            TodayView(model: model)
                .tint(.ink)
                .task { await model.startSync() }
        }
        .onChange(of: phase) { _, p in
            if p == .active { Task { await model.fetch() } }
        }
    }
}
