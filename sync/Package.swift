// swift-tools-version: 6.0
// OuroSyncKit — CKSyncEngine 을 감싼 동기화 코어. 맥 헬퍼(OuroSync.app)와 iOS 앱(개발 11~)이 같이 쓴다.
import PackageDescription

let package = Package(
    name: "OuroSyncKit",
    platforms: [.macOS("26.0"), .iOS("26.0")],
    products: [.library(name: "OuroSyncKit", targets: ["OuroSyncKit"])],
    targets: [
        .target(name: "OuroSyncKit"),
        .testTarget(name: "OuroSyncKitTests", dependencies: ["OuroSyncKit"]),
    ]
)
