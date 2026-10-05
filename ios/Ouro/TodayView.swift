// 첫 화면 — 고른 날 하루. 위에서부터: 날짜 · 주 띠 · (AI 제안 카드) · 그날 목록. 빼기가 기본이라 탭·사이드바 없이 한 화면.

import OuroSyncKit
import SwiftUI

enum Sheet: Identifiable {
    case newEvent(Date), editEvent(String), newErrand(Date), errand(String), settings
    case proposal(ProposalView), errandProposal(ErrandProposalView)
    var id: String {
        switch self {
        case .newEvent: "newEvent"
        case .editEvent(let i): "e" + i
        case .newErrand: "newErrand"
        case .errand(let i): "r" + i
        case .settings: "settings"
        case .proposal(let p): "p" + p.id
        case .errandProposal(let p): "ep" + p.id
        }
    }
}

struct TodayView: View {
    let store: RecordStore
    let refresh: () async -> Void
    @State private var day = Calendar.current.startOfDay(for: Date())
    @State private var sheet: Sheet?
    @State private var toast: String?

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
        .refreshable { await refresh() }
        .safeAreaInset(edge: .bottom) {
            if !ProcessInfo.processInfo.arguments.contains("-demo") { SyncLine(store: store) { sheet = .settings } }
        }
        .overlay(alignment: .bottom) { ToastView(text: toast) }
        .sheet(item: $sheet) { s in
            Group {
                switch s {
                case .newEvent(let d): EventSheet(store: store, editing: nil, day: d, proposal: nil)
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
                                Text("답").font(.label).foregroundStyle(Color.ai)
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
                    Text(e).foregroundStyle(Color.danger)
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
                .padding(.bottom, 48)
                .transition(.opacity.combined(with: .move(edge: .bottom)))
        }
    }
}
