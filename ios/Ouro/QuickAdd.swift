// 빠른 추가(개발 14) — 「내일 3시 치과」 한 줄을 일정으로. 날짜 계산은 맥과 같은 러스트 파서(`OuroParse`)가 하고,
// 여기선 그 결과를 시트 값으로 옮기고 «바로 넣어도 되나» 만 정한다(맥 `QuickBar`·`quick.ts` 와 같은 규칙):
//   · 날짜를 알아들었고, 제목이 있고, 경고가 없고, 이 시간대에 있는 시각이면 → 바로 넣는다.
//   · 아니면 → 시트를 열어 사람이 보고 저장한다(«추측하지 않는다», PLAN §7).
// 길은 셋: 첫 화면 아래 입력칸 · 컨트롤 버튼(입력칸이 열린 앱) · Siri·단축어(앱을 안 열고 저장, 걸리면 앱으로 넘김).

import AppIntents
import OuroParse
import OuroSyncKit
import SwiftUI

@MainActor
enum QuickAdd {
    /// 바로 넣을 수 있는 초안이면 그 값, 아니면 nil.
    static func direct(_ q: QuickDraft?) -> EventDraft? {
        guard let q, q.canDirect, let w = QuickParse.when(q) else { return nil }
        var d = EventDraft()
        d.title = q.title
        d.allDay = w.allDay
        d.start = w.start
        d.end = w.end
        return d
    }

    /// 시트로 열 값 — 알아들은 데까지 채우고, 날짜를 못 찾았으면 `day` 의 빈 시트에 제목만(맥 `quickToDraft`).
    static func sheet(_ q: QuickDraft?, text: String, day: Date) -> EventDraft {
        var d = EventSheet.blank(day: day)
        // 파서가 글을 거절했으면(NUL 등) 친 글 그대로 — 사람이 쓴 걸 잃지 않는다.
        d.title = q?.title ?? text.trimmingCharacters(in: .whitespacesAndNewlines)
        if let q, let w = QuickParse.when(q) {
            d.allDay = w.allDay
            d.start = w.start
            d.end = w.end
        }
        return d
    }

    /// 카드·Siri 가 말할 때: «내일 · 10월 7일 수요일 · 오후 3:00 – 오후 4:00».
    static func whenText(_ d: EventDraft, now: Date = Date()) -> String {
        let cal = Calendar.current
        let days = cal.dateComponents([.day], from: cal.startOfDay(for: now), to: cal.startOfDay(for: d.start)).day ?? 0
        let rel = [-1: "어제", 0: "오늘", 1: "내일", 2: "모레"][days]
        let year = cal.component(.year, from: d.start) == cal.component(.year, from: now) ? "" : "\(cal.component(.year, from: d.start))년 "
        let date = "\(year)\(Fmt.monthDay(d.start)) \(Fmt.weekday(d.start))"
        let day = rel.map { "\($0) · \(date)" } ?? date
        if d.allDay {
            if cal.isDate(d.start, inSameDayAs: d.end) { return "\(day) · 종일" }
            let n = (cal.dateComponents([.day], from: cal.startOfDay(for: d.start), to: cal.startOfDay(for: d.end)).day ?? 0) + 1
            return "\(day) – \(Fmt.monthDay(d.end)) · \(n)일"
        }
        let end = cal.isDate(d.start, inSameDayAs: d.end) ? Fmt.hm(d.end) : "\(Fmt.monthDay(d.end)) \(Fmt.hm(d.end))"
        return "\(day) · \(Fmt.hm(d.start)) – \(end)"
    }
}

/// Siri·단축어 «Ouro 에 일정 넣기». 앱을 화면 없이 깨워 저장하고 iCloud 로 올린다.
/// 🔴 바로 넣는 조건은 입력칸과 같다 — 걸리면 앱을 열어 시트로 넘긴다(목소리는 오타보다 잘못 듣기가 많다, 틀린 날짜로 조용히 저장하지 않는다).
struct AddEventIntent: AppIntent {
    static let title: LocalizedStringResource = "일정 넣기"
    static let description = IntentDescription("«내일 3시 치과» 처럼 말하면 Ouro 에 넣어요. 날짜가 애매하면 앱을 열어 확인해요.")
    /// 평소엔 화면 없이, 걸리면 앱을 열어 시트로(`continueInForeground`).
    static let supportedModes: IntentModes = [.background, .foreground(.dynamic)]

    @Parameter(title: "일정", requestValueDialog: "어떤 일정을 넣을까요?")
    var text: String

    static var parameterSummary: some ParameterSummary {
        Summary("Ouro 에 \(\.$text) 넣기")
    }

    @MainActor
    func perform() async throws -> some IntentResult & ProvidesDialog {
        let model = AppModel.shared
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw OuroError("넣을 일정을 말해 주세요") }
        let q = QuickParse.parse(text)
        guard let d = QuickAdd.direct(q) else {
            let draft = QuickAdd.sheet(q, text: text, day: Date())
            let why = q?.startDate == nil ? "날짜를 못 찾았어요" : (q?.warnings.first ?? "확인이 필요해요")
            // 사람이 «계속» 을 누르면 앱이 앞으로 나오고 여기로 돌아온다(거절하면 던져서 끝난다 — 아무것도 안 넣었다).
            try await continueInForeground(IntentDialog(stringLiteral: "\(why) — Ouro 에서 확인하고 넣어 주세요."))
            model.request = .sheet(draft)
            return .result(dialog: "확인하고 저장해 주세요.")
        }
        try model.store.createEvent(d)
        await model.pushNow()
        return .result(dialog: IntentDialog(stringLiteral: "\(QuickAdd.whenText(d)) «\(d.title)» 넣었어요."))
    }
}

struct OuroShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: AddEventIntent(),
            phrases: [
                "\(.applicationName)에 일정 넣기",
                "\(.applicationName)에 추가",
                "\(.applicationName) 일정 추가",
            ],
            shortTitle: "일정 넣기",
            systemImageName: "calendar.badge.plus")
        AppShortcut(
            intent: OpenQuickAddIntent(),
            phrases: ["\(.applicationName) 빠른 추가"],
            shortTitle: "빠른 추가",
            systemImageName: "circle.dashed")
    }
}

/// 첫 화면 아래 입력칸. 치는 동안 위에 카드가 떠서 무엇이 들어갈지 보인다(맥 `QuickBar` 와 같은 모양).
struct QuickField: View {
    @Binding var text: String
    var focused: FocusState<Bool>.Binding
    /// 보고 있는 날(오늘이 아니면) — 날짜 없는 «3시 치과» 가 그날로.
    let day: Date
    /// 바로 넣기(초안) / 시트로 열기(초안).
    let commit: (EventDraft, Bool) -> Void

    var body: some View {
        let cal = Calendar.current
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        let q = trimmed.isEmpty ? nil : QuickParse.parse(trimmed, base: cal.isDateInToday(day) ? nil : day)
        let direct = QuickAdd.direct(q)
        VStack(spacing: 8) {
            if focused.wrappedValue, !trimmed.isEmpty {
                Button { commit(QuickAdd.sheet(q, text: trimmed, day: day), false) } label: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(q?.title.isEmpty == false ? q!.title : "제목 없음")
                            .font(.bodyStrong).foregroundStyle(q?.title.isEmpty == false ? Color.ink : Color.inkMuted).lineLimit(1)
                        let shown = q.flatMap { QuickParse.when($0) != nil ? QuickAdd.sheet($0, text: trimmed, day: day) : nil }
                        Text(shown.map { QuickAdd.whenText($0) } ?? "날짜를 못 찾았어요").font(.num).foregroundStyle(Color.inkSecondary)
                        ForEach(q?.warnings ?? [], id: \.self) { Text($0).font(.caption13).foregroundStyle(Color.inkMuted) }
                        Text(direct != nil ? "완료 = 넣기 · 눌러서 고쳐 넣기" : "완료 = 확인하고 넣기")
                            .font(.micro).foregroundStyle(Color.inkMuted).padding(.top, 2)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 16).padding(.vertical, 12)
                    .background(RoundedRectangle(cornerRadius: 12).fill(Color.surface))
                    .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.hairline, lineWidth: 1))
                }
                .buttonStyle(.plain)
                .transition(.opacity)
                .accessibilityHint("시트로 열어 고쳐서 넣기")
            }
            TextField("새 일정 입력하기", text: $text)
                .font(.bodySm)
                .focused(focused)
                .submitLabel(.done)
                .onSubmit {
                    guard !trimmed.isEmpty else { return }
                    if let direct { commit(direct, true) } else { commit(QuickAdd.sheet(q, text: trimmed, day: day), false) }
                }
                .padding(.horizontal, 16).frame(height: 48)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color.sunken))
                .accessibilityLabel("빠른 입력")
                .accessibilityHint("«내일 3시 치과» 처럼 날짜와 함께 써요")
        }
        .animation(.easeOut(duration: 0.15), value: focused.wrappedValue && !trimmed.isEmpty)
    }
}
