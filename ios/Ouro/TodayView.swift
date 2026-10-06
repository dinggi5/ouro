// 첫 화면 — 고른 날 하루. 위에서부터: 날짜 · 주 띠 · (AI 제안 카드) · 그날 목록. 빼기가 기본이라 탭·사이드바 없이 한 화면.

import OuroSyncKit
import SwiftUI

enum Sheet: Identifiable {
    case newEvent(Date), editEvent(String), newErrand(Date), errand(String), settings
    /// 빠른 추가가 사람에게 넘긴 초안과 그 까닭(개발 14).
    case quick(EventDraft, [String])
    case proposal(ProposalView), errandProposal(ErrandProposalView)
    var id: String {
        switch self {
        case .newEvent: "newEvent"
        case .editEvent(let i): "e" + i
        case .newErrand: "newErrand"
        case .errand(let i): "r" + i
        case .settings: "settings"
        case .quick: "quick"
        case .proposal(let p): "p" + p.id
        case .errandProposal(let p): "ep" + p.id
        }
    }
}

struct TodayView: View {
    @Bindable var model: AppModel
    @State private var day = Calendar.current.startOfDay(for: Date())
    /// 마지막으로 본 «오늘» — 자정을 넘겨 돌아왔을 때 오늘을 보고 있었으면 새 오늘로 옮긴다(고른 날은 그대로, 코덱스 개발 16 P2).
    @State private var lastToday = Calendar.current.startOfDay(for: Date())
    @Environment(\.scenePhase) private var phase
    @State private var sheet: Sheet?
    @State private var toast: String?
    @State private var quick = ""
    @FocusState private var quickFocused: Bool

    private var store: RecordStore { model.store }

    private var cal: Calendar { .current }

    var body: some View {
        let events = store.events(on: day)
        let errands = store.errands(on: day)
        let proposals = store.pendingProposals()
        let errandProposals = store.pendingErrandProposals()
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                header
                WeekStrip(day: $day)
                    .padding(.top, 16)
                if let p = errandProposals.first {
                    ErrandProposalCard(p: p, more: errandProposals.count + proposals.count - 1, store: store, onEdit: { sheet = .errandProposal(p) }, onError: show)
                        .padding(.top, 20)
                } else if let p = proposals.first {
                    ProposalCard(p: p, more: proposals.count - 1, store: store, onEdit: { sheet = .proposal(p) }, onError: show)
                        .padding(.top, 20)
                }
                DayList(events: events, errands: errands, open: { sheet = $0 })
                    .padding(.top, 24)
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 48)
        }
        .scrollIndicators(.hidden)
        .background(Color.canvas)
        .refreshable { await model.fetch() }
        .scrollDismissesKeyboard(.immediately)
        // 아래 입력칸 위에 뜨게 — safeAreaInset 보다 먼저 붙여야 줄어든 안전 영역 안에 놓인다.
        .overlay(alignment: .bottom) { ToastView(text: toast) }
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 0) {
                QuickField(text: $quick, focused: $quickFocused, day: day, commit: quickCommit)
                    .padding(.horizontal, 20).padding(.top, 8).padding(.bottom, model.demo ? 12 : 4)
                if !model.demo { SyncLine(store: store) { sheet = .settings } }
            }
            .background(Color.canvas.opacity(0.96))
        }
        .onChange(of: model.request, initial: true) { _, r in take(r) }
        .onChange(of: phase) { _, p in if p == .active { rollToday() } }
        .onReceive(NotificationCenter.default.publisher(for: UIApplication.significantTimeChangeNotification)) { _ in rollToday() }
        .task {
            // 데모 화면 확인용: `-demo -quick «내일 3시 치과»` 면 입력칸에 그 글을 넣고 연다(시뮬레이터 자동 입력이 한글을 못 친다).
            let args = ProcessInfo.processInfo.arguments
            if model.demo, let i = args.firstIndex(of: "-quick"), i + 1 < args.count {
                quick = args[i + 1]
                take(.quickAdd)
            }
        }
        .sheet(item: $sheet) { s in
            Group {
                switch s {
                case .newEvent(let d): EventSheet(store: store, editing: nil, day: d, proposal: nil)
                case .quick(let d, let why): EventSheet(store: store, editing: nil, day: day, proposal: nil, prefill: d, notice: why, onSaved: { quick = "" })
                case .editEvent(let id): EventSheet(store: store, editing: id, day: day, proposal: nil)
                case .proposal(let p): EventSheet(store: store, editing: nil, day: day, proposal: p)
                case .newErrand(let d): ErrandSheet(store: store, editing: nil, day: d, proposal: nil)
                case .errand(let id): ErrandSheet(store: store, editing: id, day: day, proposal: nil)
                case .errandProposal(let p): ErrandSheet(store: store, editing: nil, day: day, proposal: p)
                case .settings: SettingsSheet(store: store)
                }
            }
            .presentationCornerRadius(20)
        }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            VStack(alignment: .leading, spacing: 4) {
                Text(Fmt.monthDay(day)).font(.heading).foregroundStyle(Color.ink)
                Text(Fmt.weekday(day)).font(.caption13).foregroundStyle(Color.inkMuted)
            }
            Spacer()
            if !cal.isDateInToday(day) {
                Button("오늘") { withAnimation(.easeOut(duration: 0.2)) { day = cal.startOfDay(for: Date()) } }
                    .font(.label).foregroundStyle(Color.inkSecondary)
                    .padding(.trailing, 12)
            }
            Menu {
                Button { sheet = .newEvent(day) } label: { Label("일정", systemImage: "calendar") }
                Button { sheet = .newErrand(day) } label: { Label("부탁 · AI 에게", systemImage: "circle") }
            } label: {
                Image(systemName: "plus").font(.system(size: 18, weight: .medium)).foregroundStyle(Color.ink)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel("추가")
        }
        .padding(.top, 8)
    }

    /// 컨트롤 버튼·Siri 가 보낸 부탁을 받는다. 앱이 막 켜졌으면 화면이 뜬 뒤에 온다(`initial`).
    /// 날이 바뀌었으면: 오늘을 보고 있던 화면만 새 오늘로(빠른 입력 «3시 치과» 가 어제로 가지 않게).
    private func rollToday() {
        let now = cal.startOfDay(for: Date())
        guard now != lastToday else { return }
        if day == lastToday { day = now }
        lastToday = now
    }

    private func take(_ r: AppRequest?) {
        guard let r else { return }
        model.request = nil
        switch r {
        case .quickAdd:
            sheet = nil
            // 시트가 내려가는 중이면 포커스가 안 붙는다 — 한 박자 뒤에.
            Task {
                try? await Task.sleep(for: .milliseconds(350))
                quickFocused = true
            }
        case .sheet(let d, let why):
            sheet = .quick(d, why)
        case .today:
            sheet = nil
            day = cal.startOfDay(for: Date())
        case .event(let id):
            // 그 사이 지워졌으면 그 날 대신 오늘로.
            guard let e = store.event(id) else { return take(.today) }
            day = cal.startOfDay(for: e.start)
            sheet = .editEvent(id)
        case .errand(let id):
            guard let r = store.errand(id) else { return take(.today) }
            day = cal.startOfDay(for: r.at)
            sheet = .errand(id)
        }
    }

    /// 빠른 입력 확정. 바로 넣으면 그날로 옮겨 보여 주고, 아니면 시트에서 사람이 저장할 때 입력칸을 비운다(맥과 같다).
    private func quickCommit(_ d: EventDraft, _ why: [String]?) {
        guard let why else { return add(d) }
        quickFocused = false
        sheet = .quick(d, why)
    }

    private func add(_ d: EventDraft) {
        do {
            try store.createEvent(d)
            quick = ""
            quickFocused = false
            withAnimation(.easeOut(duration: 0.2)) { day = Calendar.current.startOfDay(for: d.start) }
            show("넣었어요 · \(QuickAdd.whenText(d))")
        } catch {
            show(error.localizedDescription)
        }
    }

    private func show(_ t: String) {
        withAnimation(.easeOut(duration: 0.2)) { toast = t }
        Task {
            try? await Task.sleep(for: .seconds(3))
            withAnimation(.easeOut(duration: 0.2)) { if toast == t { toast = nil } }
        }
    }
}

struct WeekStrip: View {
    @Binding var day: Date
    private var cal: Calendar { .current }

    var body: some View {
        let start = cal.dateInterval(of: .weekOfYear, for: day)?.start ?? day
        HStack(spacing: 0) {
            Button { move(-7) } label: { Image(systemName: "chevron.left") }
                .frame(width: 28, height: 44).accessibilityLabel("지난주")
            ForEach(0..<7, id: \.self) { i in
                let d = cal.date(byAdding: .day, value: i, to: start)!
                let picked = cal.isDate(d, inSameDayAs: day)
                Button {
                    withAnimation(.easeOut(duration: 0.2)) { day = d }
                } label: {
                    VStack(spacing: 6) {
                        Text(Fmt.weekdayShort(d)).font(.micro).foregroundStyle(Color.inkMuted)
                        Text("\(cal.component(.day, from: d))")
                            .font(.num.weight(picked ? .semibold : .regular))
                            .foregroundStyle(picked ? Color.surface : (cal.isDateInToday(d) ? Color.ink : Color.inkSecondary))
                            .frame(width: 34, height: 34)
                            .background(Circle().fill(picked ? Color.ink : .clear))
                            .overlay(Circle().strokeBorder(Color.hairline, lineWidth: cal.isDateInToday(d) && !picked ? 1 : 0))
                    }
                    .frame(maxWidth: .infinity)
                }
                .accessibilityLabel(Fmt.monthDay(d))
            }
            Button { move(7) } label: { Image(systemName: "chevron.right") }
                .frame(width: 28, height: 44).accessibilityLabel("다음 주")
        }
        .font(.system(size: 13, weight: .medium))
        .foregroundStyle(Color.inkMuted)
        .buttonStyle(.plain)
    }

    private func move(_ days: Int) {
        withAnimation(.easeOut(duration: 0.2)) { day = cal.date(byAdding: .day, value: days, to: day)! }
    }
}

struct DayList: View {
    let events: [EventView]
    let errands: [ErrandView]
    let open: (Sheet) -> Void

    enum Row: Identifiable {
        case event(EventView), errand(ErrandView)
        var id: String { switch self { case .event(let e): e.id; case .errand(let e): e.id } }
        var at: Date { switch self { case .event(let e): e.start; case .errand(let e): e.at } }
    }

    var body: some View {
        let allDay = events.filter(\.allDay)
        let rows = (events.filter { !$0.allDay }.map(Row.event) + errands.map(Row.errand)).sorted { ($0.at, $0.id) < ($1.at, $1.id) }
        if allDay.isEmpty && rows.isEmpty {
            Text("비어 있어요")
                .font(.bodySm).foregroundStyle(Color.inkMuted)
                .frame(maxWidth: .infinity).padding(.top, 64)
        } else {
            VStack(spacing: 0) {
                ForEach(allDay) { e in
                    rowButton(.editEvent(e.id)) {
                        Text("종일").font(.caption13).foregroundStyle(Color.inkMuted).frame(width: 84, alignment: .leading).lineLimit(1).fixedSize(horizontal: false, vertical: true)
                        Color.clear.frame(width: 22, height: 1)
                        Text(e.title).font(.body17).foregroundStyle(Color.ink).lineLimit(1)
                    }
                }
                ForEach(rows) { r in
                    switch r {
                    case .event(let e):
                        rowButton(.editEvent(e.id)) {
                            Text(Fmt.hm(e.start)).font(.num).foregroundStyle(Color.inkMuted).frame(width: 84, alignment: .leading).lineLimit(1).fixedSize(horizontal: false, vertical: true)
                            Color.clear.frame(width: 22, height: 1)
                            Text(e.title).font(.body17).foregroundStyle(Color.ink).lineLimit(1)
                        }
                    case .errand(let e):
                        rowButton(.errand(e.id)) {
                            Text(Fmt.hm(e.at)).font(.num).foregroundStyle(Color.inkMuted).frame(width: 84, alignment: .leading).lineLimit(1).fixedSize(horizontal: false, vertical: true)
                            ErrandMark(status: e.run?.status).frame(width: 22, alignment: .leading)
                            Text(e.title).font(.body17).foregroundStyle(Color.ink).lineLimit(1)
                            Spacer(minLength: 8)
                            if let r = e.run, r.status == "done", !r.read {
                                Text("답").font(.label).foregroundStyle(Color.ai).accessibilityLabel("안 읽음")
                            }
                        }
                    }
                }
            }
        }
    }

    private func rowButton<C: View>(_ s: Sheet, @ViewBuilder _ content: () -> C) -> some View {
        Button { open(s) } label: {
            HStack(spacing: 0) { content() }
                .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct SyncLine: View {
    let store: RecordStore
    let open: () -> Void
    var body: some View {
        let s = store.sync
        Button(action: open) {
            HStack(spacing: 6) {
                if let e = s.error {
                    Text(e).foregroundStyle(Color.ink)
                } else if store.pendingCount > 0 {
                    Text("보내는 중 · \(store.pendingCount)개")
                } else if let t = s.lastSync {
                    Text("iCloud · \(Fmt.ago(t))")
                } else {
                    Text("iCloud")
                }
            }
            .font(.micro).foregroundStyle(Color.inkMuted)
            .frame(maxWidth: .infinity).frame(height: 32)
            .background(Color.canvas.opacity(0.94))
        }
        .buttonStyle(.plain)
    }
}

struct ToastView: View {
    let text: String?
    var body: some View {
        if let text {
            Text(text).font(.label).foregroundStyle(Color.surface)
                .padding(.horizontal, 16).frame(height: 44)
                .background(Capsule().fill(Color.ink))
                .padding(.bottom, 16)
                .transition(.opacity.combined(with: .move(edge: .bottom)))
        }
    }
}
