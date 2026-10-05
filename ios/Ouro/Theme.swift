// 색·글자 — 맥(src/App.css)과 같은 토큰: 生成り(키나리) 바탕 · 먹 글자 · 朱(주홍) 하나(개발 13, DESIGN.md «정체성»).
// Ouro 규칙: **색을 갖는 건 부탁·답·제안뿐**(«액센트 = AI 가 관여한 것»). 저장 버튼·고른 날짜·스위치도 먹색(ink)이고, 빨강은 없다(오류도 먹색 문장).

import SwiftUI
import UIKit

extension Color {
    static func dyn(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(UIColor { $0.userInterfaceStyle == .dark ? UIColor(hex: dark) : UIColor(hex: light) })
    }
    static let canvas = dyn(0xF7F4EC, 0x1B1A17)
    static let surface = dyn(0xFCFAF5, 0x22211D)
    static let sunken = dyn(0xEFEBE1, 0x2A2824)
    static let hairline = dyn(0xE3DED2, 0x38352F)
    static let ink = dyn(0x22201C, 0xECE7DC)
    static let inkSecondary = dyn(0x57524A, 0xA39D92)
    static let inkMuted = dyn(0x8C867B, 0x757066)
    /// 朱 — AI 가 관여한 것(부탁·답·제안)에만.
    static let ai = dyn(0xC9472B, 0xE98466)
}

extension UIColor {
    convenience init(hex: UInt32) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255,
            alpha: 1)
    }
}

extension Font {
    static let heading = Font.system(size: 24, weight: .semibold)
    static let body17 = Font.system(size: 17)
    static let bodyStrong = Font.system(size: 17, weight: .semibold)
    static let bodySm = Font.system(size: 15)
    static let label = Font.system(size: 14, weight: .medium)
    static let caption13 = Font.system(size: 13)
    static let micro = Font.system(size: 12)
    static let num = Font.system(size: 15).monospacedDigit()
}

/// 부탁 상태 표시 — 엔소(붓 한 번 원, Enso.swift): 열림 = 대기 · 앞만 짙음 = 도는 중 · 닫혀 채워짐 = 답(DESIGN.md).
/// 실패·멈춤·건너뜀은 색 없는 열린 원(끝나지 못한 고리).
struct ErrandMark: View {
    let status: String?
    var size: CGFloat = 12
    var body: some View {
        ZStack {
            switch status {
            case nil:
                EnsoShape().fill(Color.ai)
            case "running":
                EnsoShape().fill(Color.ai.opacity(0.35))
                EnsoShape().fill(Color.ai).mask(EnsoShape(points: EnsoGeometry.runClip))
            case "done":
                Circle().fill(Color.ai).padding(size * (12 - EnsoGeometry.r) / 24)
            case "failed":
                EnsoShape().fill(Color.inkSecondary)
            default:
                EnsoShape().fill(Color.inkMuted)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

enum Fmt {
    static let ko = Locale(identifier: "ko_KR")
    static func hm(_ d: Date) -> String { d.formatted(.dateTime.hour(.defaultDigits(amPM: .abbreviated)).minute().locale(ko)) }
    static func monthDay(_ d: Date) -> String {
        let c = Calendar.current.dateComponents([.month, .day], from: d)
        return "\(c.month ?? 0)월 \(c.day ?? 0)일"
    }
    static func weekday(_ d: Date) -> String { d.formatted(.dateTime.weekday(.wide).locale(ko)) }
    static func weekdayShort(_ d: Date) -> String { d.formatted(.dateTime.weekday(.narrow).locale(ko)) }
    static func ago(_ d: Date, now: Date = Date()) -> String {
        let s = now.timeIntervalSince(d)
        if s < 60 { return "방금" }
        if s < 3600 { return "\(Int(s / 60))분 전" }
        return hm(d)
    }
    static func target(_ t: String) -> String { t == "codex" ? "Codex" : "Claude Code" }
    static func client(_ c: String) -> String {
        let l = c.lowercased()
        if l.contains("claude") { return "Claude" }
        if l.contains("codex") || l.contains("gpt") { return "Codex" }
        return "AI"
    }
}
