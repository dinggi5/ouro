// swift-tools-version: 6.0
// OuroParse — 맥의 러스트 날짜 파서(`ouro-parse`)를 스위프트에서 부른다(개발 14). iOS 앱의 빠른 추가·Siri 가 쓴다.
// 바이너리는 `scripts/build-parse.sh` 가 만든다(커밋하지 않는다) — 없으면 이 패키지는 못 푼다.
import PackageDescription

let package = Package(
    name: "OuroParse",
    platforms: [.macOS("26.0"), .iOS("26.0")],
    products: [.library(name: "OuroParse", targets: ["OuroParse"])],
    targets: [
        .binaryTarget(name: "OuroParseFFI", path: "OuroParseFFI.xcframework"),
        .target(name: "OuroParse", dependencies: ["OuroParseFFI"]),
        .testTarget(name: "OuroParseTests", dependencies: ["OuroParse"]),
    ]
)
