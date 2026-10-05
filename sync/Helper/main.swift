// OuroSync.app — Ouro 가 띄우는 동기화 헬퍼(Contents/Helpers). 화면 없음(LSUIElement).
//
// 왜 따로 된 앱인가: CloudKit 권한(entitlement)은 프로비저닝 프로파일이 든 **번들**에만 붙는다. Tauri 본체에 Swift 를 섞는 대신
// 헬퍼 번들 하나가 CloudKit·푸시를 맡고, 본체와는 표준입력·출력 JSON 줄로만 말한다. DB 는 본체만 쓴다(MCP 사이드카와 같은 원칙).
//
// 쓰는 법: OuroSync --state <폴더> [--container iCloud.com.dinggi5.ouro]
// 표준입력이 닫히면(본체가 끝나거나 죽으면) 같이 끝난다 — 고아로 남지 않는다.

import AppKit
import Foundation
import OuroSyncKit

final class StdoutOutlet: Outlet, @unchecked Sendable {
    let lock = NSLock()
    func send(_ msg: Outbound) {
        guard var data = try? JSONEncoder().encode(msg) else { return }
        data.append(0x0A)
        lock.lock()
        FileHandle.standardOutput.write(data)
        lock.unlock()
    }
}

func arg(_ name: String) -> String? {
    let a = CommandLine.arguments
    guard let i = a.firstIndex(of: name), i + 1 < a.count else { return nil }
    return a[i + 1]
}

guard let state = arg("--state") else {
    FileHandle.standardError.write(Data("--state <폴더> 가 필요해요\n".utf8))
    exit(2)
}
let stateDir = URL(fileURLWithPath: state, isDirectory: true)
try? FileManager.default.createDirectory(at: stateDir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
let core = SyncCore(
    containerID: arg("--container") ?? "iCloud.com.dinggi5.ouro", stateDir: stateDir, out: StdoutOutlet())

Task.detached {
    await core.start()
    do {
        for try await line in FileHandle.standardInput.bytes.lines {
            guard let m = try? JSONDecoder().decode(Inbound.self, from: Data(line.utf8)) else { continue }
            if m.op == "quit" { break }
            // 순서를 지키되 답(records·ack)이 막히지 않게: 답은 기다리는 쪽을 깨우기만 하므로 바로 처리하고,
            // fetch·send 처럼 오래 걸리는 것은 따로 띄운다.
            if m.op == "fetch" || m.op == "send" {
                Task { await core.receive(m) }
            } else {
                await core.receive(m)
            }
        }
    } catch {}
    exit(0)
}

// 푸시 = 다른 기기가 바꾸면 곧바로 받기. 못 받아도 본체가 주기적으로 fetch 를 보낸다.
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
app.registerForRemoteNotifications()
app.run()
