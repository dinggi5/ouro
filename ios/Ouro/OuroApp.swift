// Ouro iOS — 맥의 Ouro 와 iCloud 로 오가는 캘린더(개발 11). 보기·쓰기·부탁 걸기·답 읽기·AI 제안 승인.
// 부탁은 폰이 돌리지 않는다 — «실행 맥» 이 돌리고 답이 동기화로 온다.

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

@MainActor
final class AppModel {
    let store: RecordStore
    let bridge: LocalBridge

    init() {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Ouro")
        let demo = ProcessInfo.processInfo.arguments.contains("-demo")
        store = RecordStore(url: demo ? nil : base.appendingPathComponent("records.json"))
        bridge = LocalBridge(store: store, stateDir: base.appendingPathComponent("sync"))
        if demo { Demo.fill(store) }
    }
}

@main
struct OuroApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var delegate
    @Environment(\.scenePhase) private var phase
    @State private var model = AppModel()
    @State private var started = false

    var body: some Scene {
        WindowGroup {
            TodayView(store: model.store, refresh: { await model.bridge.fetch() })
                .tint(.ink)
                .task {
                    guard !started, !ProcessInfo.processInfo.arguments.contains("-demo") else { return }
                    started = true
                    await model.bridge.start()
                }
        }
        .onChange(of: phase) { _, p in
            if p == .active, started { Task { await model.bridge.fetch() } }
        }
    }
}
