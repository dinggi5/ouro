// 다음 일정 위젯(개발 15) — 홈 화면 작게·중간, 잠금 화면 사각·한 줄. 답 위젯 — 잠금 화면 원.
// 앱이 쓴 피드(`WidgetFeed`)만 읽는다. 색은 부탁·답에만(朱), 일정은 먹색 — 앱과 같은 규칙(DESIGN.md «액센트»).

import SwiftUI
import WidgetKit

struct FeedEntry: TimelineEntry {
    let date: Date
    let feed: WidgetFeed
}

struct FeedProvider: TimelineProvider {
    func placeholder(in context: Context) -> FeedEntry { FeedEntry(date: Date(), feed: .sample) }

    func getSnapshot(in context: Context, completion: @escaping (FeedEntry) -> Void) {
        // 위젯 고르는 화면에선 비어 있으면 예시로 — 무엇을 보여 주는 위젯인지 알게.
        let feed = WidgetFeed.load() ?? WidgetFeed()
        completion(FeedEntry(date: Date(), feed: context.isPreview && feed.items.isEmpty && feed.unread == 0 ? .sample : feed))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<FeedEntry>) -> Void) {
        let now = Date()
        let feed = WidgetFeed.load() ?? WidgetFeed()
        let entries = ([now] + feed.changes(after: now)).map { FeedEntry(date: $0, feed: feed) }
        // 마지막 칸이 지나면 피드를 다시 읽는다(앱이 그 사이 새로 썼을 수 있다).
        completion(Timeline(entries: entries, policy: .atEnd))
    }
}

extension WidgetFeed {
    static var sample: WidgetFeed {
        let cal = Calendar.current
        let now = Date()
        let h = cal.component(.hour, from: now)
        func at(_ hour: Int) -> Date { cal.date(bySettingHour: min(hour, 23), minute: 0, second: 0, of: now)! }
        return WidgetFeed(
            items: [
                .init(id: "a", title: "치과", allDay: false, start: at(h + 1), end: at(h + 2), errand: false),
                .init(id: "b", title: "회의 안건 초안", allDay: false, start: at(h + 3), end: at(h + 3), errand: true),
                .init(id: "c", title: "팀 회의", allDay: false, start: at(h + 4), end: at(h + 5), errand: false),
            ],
            answers: [.init(errand: "d", title: "어제 커밋 정리", at: now)], unread: 1)
    }
}

/// 날짜·시각 말.
enum Say {
    static func day(_ i: WidgetFeed.Item, now: Date) -> String {
        let cal = Calendar.current
        // 이미 시작한 여러 날 일정은 «오늘».
        let d = max(i.start, cal.startOfDay(for: now))
        if cal.isDate(d, inSameDayAs: now) { return "오늘" }
        if let t = cal.date(byAdding: .day, value: 1, to: now), cal.isDate(d, inSameDayAs: t) { return "내일" }
        return "\(Fmt.monthDay(d)) \(Fmt.weekdayShort(d))"
    }

    static func time(_ i: WidgetFeed.Item, now: Date) -> String {
        if i.allDay { return "종일" }
        if !i.errand, i.start <= now, now < i.end {
            return Calendar.current.isDate(i.end, inSameDayAs: now) ? "지금 · \(Fmt.hm(i.end))까지" : "지금"
        }
        return Fmt.hm(i.start)
    }

    /// 잠금 화면 한 줄: «오후 3:00 치과» · 오늘이 아니면 «내일 오후 3:00 치과».
    static func line(_ i: WidgetFeed.Item, now: Date) -> String {
        let d = day(i, now: now)
        return (d == "오늘" ? "" : d + " ") + time(i, now: now) + " " + i.title
    }
}

// MARK: 다음 일정

struct NextWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WidgetFeed.kind, provider: FeedProvider()) { e in
            NextView(entry: e).containerBackground(Color.canvas, for: .widget)
        }
        .configurationDisplayName("다음 일정")
        .description("다가오는 일정과 부탁, 도착한 답.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular, .accessoryInline])
    }
}

struct NextView: View {
    let entry: FeedEntry
    @Environment(\.widgetFamily) private var family

    private var items: [WidgetFeed.Item] { entry.feed.items(at: entry.date) }

    var body: some View {
        switch family {
        case .accessoryInline: inline
        case .accessoryRectangular: rectangular
        case .systemMedium: medium
        default: small
        }
    }

    /// 작게의 주인공 — 다음 시각 일정·부탁. 종일 일정은 하루 내내 «다음» 자리를 막으니 시각 있는 것이 없을 때만.
    private var hero: WidgetFeed.Item? { items.first { !$0.allDay } ?? items.first }

    /// 주인공 날의 종일 일정 — 위 줄에 «오늘 · 엄마 생신» 으로.
    private func allDayLine(_ h: WidgetFeed.Item) -> String {
        let day = Say.day(h, now: entry.date)
        guard !h.allDay, let a = items.first(where: { $0.allDay && Say.day($0, now: entry.date) == day }) else { return day }
        return "\(day) · \(a.title)"
    }

    // 작게 — 다음 하나를 크게. 아래 줄은 답(있으면) 또는 그다음 것.
    private var small: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let i = hero {
                Text(allDayLine(i)).font(.caption13).foregroundStyle(Color.inkMuted).lineLimit(1).privacySensitive()
                Spacer(minLength: 6)
                HStack(spacing: 5) {
                    if i.errand { ErrandMark(status: nil, size: 10, spoken: false) }
                    Text(Say.time(i, now: entry.date)).font(.caption13.monospacedDigit()).foregroundStyle(i.errand ? Color.ai : Color.inkSecondary)
                        .lineLimit(1).minimumScaleFactor(0.8)
                }
                Text(i.title).font(.bodyStrong).foregroundStyle(Color.ink).lineLimit(3).privacySensitive()
                    .padding(.top, 2)
                Spacer(minLength: 6)
                footer(next: items.first { $0 != i && !$0.allDay && $0.start >= i.start })
            } else {
                Text("오늘").font(.caption13).foregroundStyle(Color.inkMuted)
                Spacer()
                Text("다가오는 일정이 없어요").font(.bodySm).foregroundStyle(Color.inkMuted)
                Spacer()
                footer(next: nil)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(hero.map(OuroLink.item) ?? entry.feed.answers.first.map { OuroLink.errand($0.errand) } ?? OuroLink.today)
    }

    @ViewBuilder private func footer(next: WidgetFeed.Item?) -> some View {
        if entry.feed.unread > 0 {
            answerBadge
        } else if let n = next {
            Text("다음 · \(Say.time(n, now: entry.date)) \(n.title)").privacySensitive().font(.micro).foregroundStyle(Color.inkMuted).lineLimit(1)
        }
    }

    private var answerBadge: some View {
        HStack(spacing: 5) {
            ErrandMark(status: "done", size: 10, spoken: false)
            Text(entry.feed.unread == 1 ? "답 도착" : "답 \(entry.feed.unread)개").font(.micro).foregroundStyle(Color.ai)
        }
    }

    // 중간 — 머리(오늘 날짜 · 답) + 셋까지. 날이 바뀌는 행에만 날을 적는다.
    private var medium: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline) {
                Text("\(Fmt.monthDay(entry.date)) \(Fmt.weekday(entry.date))").font(.caption13).foregroundStyle(Color.inkMuted)
                Spacer()
                if let a = entry.feed.answers.first, entry.feed.unread > 0 {
                    Link(destination: OuroLink.errand(a.errand)) { answerBadge }
                }
            }
            if items.isEmpty {
                Spacer()
                Text("다가오는 일정이 없어요").font(.bodySm).foregroundStyle(Color.inkMuted)
                Spacer()
            } else {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(rows, id: \.0.id) { i, day in
                        Link(destination: OuroLink.item(i)) { row(i, day: day) }
                    }
                }
                .padding(.top, 12)
                Spacer(minLength: 0)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(OuroLink.today)
    }

    /// 중간의 행들 — 날이 바뀌는 행에만 날(두 줄이 된다). 날 줄이 없으면 넷, 있으면 셋까지.
    private var rows: [(WidgetFeed.Item, String?)] {
        var out: [(WidgetFeed.Item, String?)] = []
        var prev = "오늘"
        for i in items {
            let day = Say.day(i, now: entry.date)
            out.append((i, day == prev ? nil : day))
            prev = day
        }
        return Array(out.prefix(out.prefix(4).contains { $0.1 != nil } ? 3 : 4))
    }

    private func row(_ i: WidgetFeed.Item, day: String?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 1) {
                if let day { Text(day).font(.micro).foregroundStyle(Color.inkMuted) }
                Text(Say.time(i, now: entry.date)).font(.caption13.monospacedDigit())
                    .foregroundStyle(i.errand ? Color.ai : Color.inkSecondary).lineLimit(1).minimumScaleFactor(0.7)
            }
            .frame(width: 76, alignment: .leading)
            HStack(spacing: 6) {
                if i.errand { ErrandMark(status: nil, size: 10, spoken: false) }
                Text(i.title).font(.bodySm).foregroundStyle(Color.ink).lineLimit(1).privacySensitive()
            }
        }
    }

    // 잠금 화면 — 색은 시스템이 입힌다. 시각 한 줄 + 제목.
    private var rectangular: some View {
        VStack(alignment: .leading, spacing: 1) {
            if let i = hero {
                let d = Say.day(i, now: entry.date)
                Text((d == "오늘" ? "" : d + " ") + Say.time(i, now: entry.date)).font(.headline).widgetAccentable().lineLimit(1)
                Text(i.title).font(.body).lineLimit(2).privacySensitive()
            } else {
                Text("다가오는 일정 없음").font(.headline).widgetAccentable()
            }
            if entry.feed.unread > 0 {
                Text(entry.feed.unread == 1 ? "답 도착" : "답 \(entry.feed.unread)개").font(.caption).foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .widgetURL(hero.map(OuroLink.item) ?? OuroLink.today)
    }

    private var inline: some View {
        Group {
            if let i = hero { Text(Say.line(i, now: entry.date)) } else { Text("다가오는 일정 없음") }
        }
        .privacySensitive()
        .widgetURL(hero.map(OuroLink.item) ?? OuroLink.today)
    }
}

// MARK: 답 — 잠금 화면 원. 열린 엔소 = 없음, 닫힌 원 + 수 = 도착(우로보로스).

struct AnswersWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WidgetFeed.answersKind, provider: FeedProvider()) { e in
            AnswersView(feed: e.feed).containerBackground(.clear, for: .widget)
        }
        .configurationDisplayName("답")
        .description("AI 에게 건 부탁의 답이 오면 원이 닫힙니다.")
        .supportedFamilies([.accessoryCircular])
    }
}

struct AnswersView: View {
    let feed: WidgetFeed
    var body: some View {
        ZStack {
            AccessoryWidgetBackground()
            if feed.unread > 0 {
                Circle().strokeBorder(lineWidth: 3).padding(6)
                Text("\(feed.unread)").font(.system(size: 20, weight: .semibold).monospacedDigit())
            } else {
                EnsoShape().padding(6).opacity(0.6)
            }
        }
        .widgetAccentable()
        .accessibilityElement()
        .accessibilityLabel(feed.unread > 0 ? "답 \(feed.unread)개 도착" : "새 답 없음")
        .widgetURL(feed.answers.first.map { OuroLink.errand($0.errand) } ?? OuroLink.today)
    }
}
