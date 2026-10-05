// 시트들 — 일정 · 부탁(+답) · AI 제안 고치기 · 설정. 시스템 Form 위에 먹색 저장 버튼 하나(액센트는 부탁·답만).

import OuroSyncKit
import SwiftUI

private let alertLabels: [(Int64?, String)] = [
    (nil, "없음"), (0, "정각"), (5, "5분 전"), (10, "10분 전"), (15, "15분 전"), (30, "30분 전"), (60, "1시간 전"), (120, "2시간 전"), (1440, "하루 전"),
]
private let allDayAlertLabels: [(Int64?, String)] = [(nil, "없음"), (0, "당일 오전 9시"), (1440, "전날 오전 9시")]

struct SheetFrame<Content: View>: View {
    let title: String
    let saveLabel: String
    let canSave: Bool
    var error: String?
    let save: () -> Void
    @ViewBuilder let content: Content
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form { content }
                .scrollContentBackground(.hidden)
                .background(Color.canvas)
                .navigationTitle(title)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("닫기") { dismiss() }.foregroundStyle(Color.inkSecondary) }
                    if !saveLabel.isEmpty {
                        ToolbarItem(placement: .confirmationAction) {
                            Button(saveLabel, action: save).fontWeight(.semibold).foregroundStyle(Color.ink).disabled(!canSave)
                        }
                    }
                }
                .safeAreaInset(edge: .bottom) {
                    if let error {
                        Text(error).font(.caption13).foregroundStyle(Color.danger).padding(.bottom, 8)
                    }
                }
        }
    }
}

struct EventSheet: View {
    let store: RecordStore
    let editing: String?
    let proposal: ProposalView?
    @State private var d: EventDraft
    @State private var error: String?
    @Environment(\.dismiss) private var dismiss

    init(store: RecordStore, editing: String?, day: Date, proposal: ProposalView?) {
        self.store = store
        self.editing = editing
        self.proposal = proposal
        var d = EventDraft()
        if let p = proposal {
            d = p.event
        } else if let id = editing, let e = store.event(id) {
            d.title = e.title
            d.notes = e.notes
            d.allDay = e.allDay
            d.start = e.start
            d.end = e.allDay ? Calendar.current.date(byAdding: .day, value: -1, to: e.end)! : e.end
            d.alertMin = e.alertMin
            d.isPrivate = e.isPrivate
        } else {
            // 오늘이면 다음 정각, 다른 날이면 오전 9시(맥과 같다).
            let cal = Calendar.current
            let now = Date()
            let start = cal.isDateInToday(day)
                ? cal.nextDate(after: now, matching: DateComponents(minute: 0), matchingPolicy: .nextTime)!
                : cal.date(bySettingHour: 9, minute: 0, second: 0, of: day)!
            d.start = start
            d.end = start.addingTimeInterval(3600)
        }
        _d = State(initialValue: d)
    }

    var body: some View {
        SheetFrame(
            title: proposal != nil ? "\(Fmt.client(proposal!.client)) 의 제안" : (editing == nil ? "새 일정" : "일정"),
            saveLabel: proposal != nil ? "승인" : "저장", canSave: !d.title.trimmingCharacters(in: .whitespaces).isEmpty, error: error, save: save
        ) {
            Section {
                TextField("제목", text: $d.title).font(.body17)
            }
            Section {
                Toggle("종일", isOn: $d.allDay.animation(.easeOut(duration: 0.2)))
                    .tint(.ink)
                    .onChange(of: d.allDay) { _, all in
                        d.alertMin = nil
                        if !all, d.end <= d.start { d.end = d.start.addingTimeInterval(3600) }
                    }
                DatePicker("시작", selection: $d.start, displayedComponents: d.allDay ? .date : [.date, .hourAndMinute])
                    .onChange(of: d.start) { old, new in
                        // 시작을 옮기면 길이를 지킨 채 끝도 따라간다.
                        d.end = d.end.addingTimeInterval(new.timeIntervalSince(old))
                    }
                DatePicker("끝", selection: $d.end, in: d.start..., displayedComponents: d.allDay ? .date : [.date, .hourAndMinute])
            }
            Section {
                Picker("알림", selection: $d.alertMin) {
                    ForEach(d.allDay ? allDayAlertLabels : alertLabels, id: \.1) { Text($0.1).tag($0.0) }
                }
                Toggle("AI 에게 숨기기", isOn: $d.isPrivate).tint(.ink)
            } footer: {
                Text("숨긴 일정은 Claude·Codex 가 읽지 못해요. 그 시간이 바쁘다는 것만 알아요.").font(.micro)
            }
            Section {
                TextField("메모", text: $d.notes, axis: .vertical).lineLimit(3...8)
            }
            if let id = editing {
                Section {
                    Button("일정 지우기", role: .destructive) {
                        try? store.trash(id)
                        dismiss()
                    }
                }
            }
            if let p = proposal {
                Section {
                    Button("거절", role: .destructive) {
                        try? store.rejectProposal(p.id)
                        dismiss()
                    }
                }
            }
        }
    }

    private func save() {
        do {
            if let p = proposal {
                try store.approveProposal(p.id, edited: d)
            } else if let id = editing {
                try store.updateEvent(id, d)
            } else {
                try store.createEvent(d)
            }
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct ErrandSheet: View {
    let store: RecordStore
    let editing: String?
    let proposal: ErrandProposalView?
    @State private var d: ErrandDraft
    @State private var error: String?
    @State private var openRun: RunView?
    @Environment(\.dismiss) private var dismiss

    init(store: RecordStore, editing: String?, day: Date, proposal: ErrandProposalView?) {
        self.store = store
        self.editing = editing
        self.proposal = proposal
        var d = ErrandDraft()
        let cal = Calendar.current
        if let p = proposal {
            d.prompt = p.prompt
            d.at = p.at
            d.target = p.target
        } else if let id = editing, let e = store.errand(id) {
            d.prompt = e.prompt
            d.at = e.at
            d.target = e.target
            d.late = e.late
            d.repeatRule = e.repeatRule
        } else {
            // 지난 때로 걸면 만들자마자 실행 맥이 돌린다 — 기본은 다음 정각, 다른 날의 미래면 오전 9시.
            let now = Date()
            let nine = cal.date(bySettingHour: 9, minute: 0, second: 0, of: day)!
            d.at = nine > now && !cal.isDateInToday(day) ? nine : cal.nextDate(after: now, matching: DateComponents(minute: 0), matchingPolicy: .nextTime)!
        }
        _d = State(initialValue: d)
    }

    private var runs: [RunView] { editing.map { store.runs(of: $0) } ?? [] }
    private var locked: Bool { !runs.isEmpty }

    var body: some View {
        SheetFrame(
            title: proposal != nil ? "\(Fmt.client(proposal!.client)) 의 부탁 제안" : (editing == nil ? "새 부탁" : "부탁"),
            saveLabel: proposal != nil ? "승인" : (locked ? "" : "저장"),
            canSave: !locked && !d.prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, error: error, save: save
        ) {
            if let latest = runs.first {
                Section {
                    AnswerBlock(run: latest)
                    if runs.count > 1 {
                        ForEach(runs.dropFirst()) { r in
                            Button { openRun = r } label: {
                                HStack {
                                    Text(Date(ms: r.startedAt).formatted(.dateTime.month().day().hour().minute().locale(Fmt.ko)))
                                        .font(.num).foregroundStyle(Color.inkSecondary)
                                    Spacer()
                                    Text(statusText(r)).font(.caption13).foregroundStyle(Color.inkMuted)
                                }
                            }
                        }
                    }
                } header: {
                    Text("답").foregroundStyle(Color.ai)
                }
                .onAppear { if latest.status == "done" { store.markRead(latest.name) } }
            }
            Section {
                if locked {
                    Text(d.prompt).font(.bodySm).foregroundStyle(Color.inkSecondary).textSelection(.enabled)
                } else {
                    TextField("Claude 에게 부탁할 말", text: $d.prompt, axis: .vertical).lineLimit(4...12)
                }
            } footer: {
                Text(locked ? "이미 보낸 부탁은 고칠 수 없어요 — 보낸 글 그대로 남아요." : "이 글만 바깥(\(Fmt.target(d.target)))으로 나가요. 부탁은 «실행 맥» 이 돌리고 답은 여기로 와요.")
                    .font(.micro)
            }
            Section {
                DatePicker("언제", selection: $d.at, displayedComponents: [.date, .hourAndMinute]).disabled(locked)
                Picker("누구에게", selection: $d.target) {
                    Text("Claude Code").tag("claude")
                    Text("Codex").tag("codex")
                }
                .disabled(locked)
                if proposal == nil {
                    Picker("반복", selection: $d.repeatRule) {
                        Text("안 함").tag("")
                        Text("매일").tag("FREQ=DAILY")
                        Text("평일").tag("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
                        Text("매주").tag("FREQ=WEEKLY")
                    }
                    .disabled(locked)
                }
                Picker("맥이 꺼져 있었으면", selection: $d.late) {
                    Text("늦게라도 실행").tag("run")
                    Text("건너뛰기").tag("skip")
                }
                .disabled(locked)
            }
            if let id = editing {
                Section {
                    Button("부탁 지우기", role: .destructive) {
                        try? store.trash(id)
                        dismiss()
                    }
                }
            }
            if let p = proposal {
                Section {
                    Button("거절", role: .destructive) {
                        try? store.rejectProposal(p.id)
                        dismiss()
                    }
                }
            }
        }
        .sheet(item: $openRun) { r in
            NavigationStack {
                ScrollView { AnswerBlock(run: r).padding(20) }
                    .background(Color.canvas)
                    .navigationTitle("지난 답").navigationBarTitleDisplayMode(.inline)
            }
            .presentationDetents([.medium, .large])
        }
    }

    private func statusText(_ r: RunView) -> String {
        switch r.status {
        case "done": "답"
        case "running": "도는 중"
        case "skipped": "건너뜀"
        case "stopped": "멈춤"
        default: "실패"
        }
    }

    private func save() {
        do {
            if let p = proposal {
                try store.approveErrandProposal(p.id, edited: d)
            } else if let id = editing {
                try store.updateErrand(id, d)
            } else {
                try store.createErrand(d)
            }
            dismiss()
        } catch {
            self.error = error.localizedDescription
        }
    }
}

struct AnswerBlock: View {
    let run: RunView
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch run.status {
            case "running":
                Label("맥에서 도는 중이에요", systemImage: "circle.dotted").font(.bodySm).foregroundStyle(Color.ai)
            case "done":
                Text(run.response ?? "").font(.bodySm).foregroundStyle(Color.ink).textSelection(.enabled)
            case "skipped":
                Text(run.stderr ?? "건너뛰었어요").font(.bodySm).foregroundStyle(Color.inkMuted)
            default:
                Text(run.stderr ?? "실패했어요").font(.bodySm).foregroundStyle(Color.inkMuted)
            }
            if run.lateMs > 60_000 {
                Text("\(run.lateMs / 60_000)분 늦게 실행됨").font(.micro).foregroundStyle(Color.inkMuted)
            }
        }
        .padding(.vertical, 4)
    }
}

/// AI 제안 카드 — 목록 위에 한 장. 승인하면 그제야 일정·부탁이 된다.
struct ProposalCard: View {
    let p: ProposalView
    let more: Int
    let store: RecordStore
    let onEdit: () -> Void
    let onError: (String) -> Void

    var body: some View {
        CardFrame(label: "\(Fmt.client(p.client)) 가 일정을 제안했어요", more: more) {
            Text(p.event.title).font(.bodyStrong).foregroundStyle(Color.ink)
            Text(p.event.allDay ? "\(Fmt.monthDay(p.event.start)) \(Fmt.weekday(p.event.start)) · 종일" : "\(Fmt.monthDay(p.event.start)) \(Fmt.weekday(p.event.start)) · \(Fmt.hm(p.event.start))")
                .font(.num).foregroundStyle(Color.inkSecondary)
        } actions: {
            Button("승인") { run { try store.approveProposal(p.id) } }.buttonStyle(PrimaryButton())
            Button("고치기", action: onEdit).buttonStyle(QuietButton())
            Button("거절") { run { try store.rejectProposal(p.id) } }.buttonStyle(QuietButton())
        }
    }

    private func run(_ f: () throws -> Void) {
        do { try f() } catch { onError(error.localizedDescription) }
    }
}

struct ErrandProposalCard: View {
    let p: ErrandProposalView
    let more: Int
    let store: RecordStore
    let onEdit: () -> Void
    let onError: (String) -> Void

    var body: some View {
        CardFrame(label: "\(Fmt.client(p.client)) 가 부탁을 제안했어요", more: more) {
            Text(p.prompt).font(.bodySm).foregroundStyle(Color.ink).lineLimit(4)
            Text("\(Fmt.monthDay(p.at)) \(Fmt.hm(p.at)) · \(Fmt.target(p.target)) 에게").font(.num).foregroundStyle(Color.inkSecondary)
        } actions: {
            Button("승인") { run { try store.approveErrandProposal(p.id) } }.buttonStyle(PrimaryButton())
            Button("고치기", action: onEdit).buttonStyle(QuietButton())
            Button("거절") { run { try store.rejectProposal(p.id) } }.buttonStyle(QuietButton())
        }
    }

    private func run(_ f: () throws -> Void) {
        do { try f() } catch { onError(error.localizedDescription) }
    }
}

struct CardFrame<Body: View, Actions: View>: View {
    let label: String
    let more: Int
    @ViewBuilder let content: Body
    @ViewBuilder let actions: Actions
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(label).font(.caption13).foregroundStyle(Color.ai)
                Spacer()
                if more > 0 { Text("+\(more)").font(.caption13).foregroundStyle(Color.inkMuted) }
            }
            content
            HStack(spacing: 8) { actions }.padding(.top, 10)
        }
        .padding(20)
        .background(RoundedRectangle(cornerRadius: 16).fill(Color.surface))
    }
}

struct PrimaryButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.label.weight(.semibold)).foregroundStyle(Color.surface)
            .frame(maxWidth: .infinity).frame(height: 44)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.ink))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}

struct QuietButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(.label).foregroundStyle(Color.inkSecondary)
            .padding(.horizontal, 14).frame(height: 44)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color.sunken))
            .opacity(configuration.isPressed ? 0.8 : 1)
    }
}

struct SettingsSheet: View {
    let store: RecordStore
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    LabeledContent("iCloud", value: accountText)
                    if let t = store.sync.lastSync { LabeledContent("마지막으로 맞춤", value: Fmt.ago(t)) }
                    if store.pendingCount > 0 { LabeledContent("보낼 것", value: "\(store.pendingCount)개") }
                    if let e = store.sync.error { Text(e).foregroundStyle(Color.danger) }
                } footer: {
                    Text("일정·부탁·답이 내 iCloud 로 내 기기끼리 오가요. 제목과 글은 암호화 칸에 담겨요 — «고급 데이터 보호» 를 켜 두면 애플도 못 읽어요.")
                        .font(.micro)
                }
                Section {
                    LabeledContent("예약 부탁을 돌리는 맥", value: store.runner?.name ?? "아직 없음")
                } footer: {
                    Text("폰은 부탁을 돌리지 않아요. 맥의 Ouro 가 «실행 맥» 으로 돌리고, 답이 여기로 와요. 실행 맥은 맥의 메뉴바 «iCloud 동기화…» 에서 바꿔요.")
                        .font(.micro)
                }
            }
            .scrollContentBackground(.hidden)
            .background(Color.canvas)
            .navigationTitle("동기화").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("닫기") { dismiss() }.foregroundStyle(Color.ink) } }
        }
    }

    private var accountText: String {
        switch store.sync.account {
        case "available": "켜짐"
        case "noAccount": "로그인 안 됨"
        case "restricted": "막혀 있음"
        case nil: "확인하는 중"
        default: "확인 못 함"
        }
    }
}
