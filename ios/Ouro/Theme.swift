// 색·글자 — 전역 DESIGN.md 토큰(토스 그레이) + Ouro 규칙: **색을 갖는 건 부탁·답뿐**(DESIGN.md «액센트 = AI 가 관여한 것»).
// 저장 버튼·고른 날짜·스위치도 먹색(ink)이다.

import SwiftUI
import UIKit

extension Color {
    static func dyn(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(UIColor { $0.userInterfaceStyle == .dark ? UIColor(hex: dark) : UIColor(hex: light) })
    }
    static let canvas = dyn(0xF9FAFB, 0x151517)
    static let surface = dyn(0xFFFFFF, 0x1C1C1F)
    static let sunken = dyn(0xF2F4F6, 0x242428)
    static let hairline = dyn(0xE5E8EB, 0x33333A)
    static let ink = dyn(0x191F28, 0xF2F2F4)
    static let inkSecondary = dyn(0x4E5968, 0x9A9AA1)
    static let inkMuted = dyn(0x8B95A1, 0x6E6E73)
    /// AI 가 관여한 것(부탁·답·제안)에만.
    static let ai = dyn(0x3182F6, 0x64A8FF)
    static let aiTint = dyn(0xE8F3FF, 0x1E2A3A)
    static let danger = dyn(0xF04452, 0xF04452)
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

/// 부탁 상태 표시 — ◯ 대기 · ◔ 실행 중 · ● 답 도착(DESIGN.md).
struct ErrandMark: View {
    let status: String?
    var body: some View {
        ZStack {
            switch status {
            case nil:
                Circle().strokeBorder(Color.ai, lineWidth: 1.5)
            case "running":
                Circle().strokeBorder(Color.ai, lineWidth: 1.5)
                Circle().trim(from: 0, to: 0.25).fill(Color.ai).rotationEffect(.degrees(-90)).padding(1)
            case "done":
                Circle().fill(Color.ai)
            default:
                Circle().strokeBorder(Color.inkMuted, lineWidth: 1.5)
            }
        }
        .frame(width: 12, height: 12)
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
