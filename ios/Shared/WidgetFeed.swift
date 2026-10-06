// 위젯 피드(개발 15) — 앱이 저장소가 바뀔 때마다 만들어 앱 그룹 폴더에 두는 작은 JSON. 위젯은 이것만 읽는다.
// 저장소(`records.json`)를 앱 그룹으로 옮기지 않은 까닭: 옮기면 기존 설치의 파일을 이사해야 하고, 확장이 같은 파일을
// 열면 쓰는 쪽이 둘이 될 수 있다. 피드는 앱만 쓰고 위젯은 읽기만 한다 — 위젯에 필요한 몇 줄만 나간다.
// 앱과 위젯 확장 **둘 다**에 들어간다.

import Foundation

struct WidgetFeed: Codable, Equatable {
    struct Item: Codable, Equatable {
        var id: String
        var title: String
        var allDay: Bool
        var start: Date
        /// 일정의 끝(종일이면 배타). 부탁은 `start` 와 같다 — 시각이 되면 «도는 중» 이라 다가오는 것에서 빠진다.
        var end: Date
        var errand: Bool
    }

    struct Answer: Codable, Equatable {
        var errand: String
        var title: String
        var at: Date
    }

    /// 다가오는 것(7일 안, 시작순).
    var items: [Item] = []
    /// 안 읽은 답 — 최근 것부터 몇 개만.
    var answers: [Answer] = []
    /// 안 읽은 답 전부의 수.
    var unread = 0

    static let group = "group.com.dinggi5.ouro"
    static let kind = "com.dinggi5.ouro.ios.next"
    static let answersKind = "com.dinggi5.ouro.ios.answers"

    static var url: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group)?.appendingPathComponent("widget.json")
    }

    static func load() -> WidgetFeed? {
        guard let url, let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(WidgetFeed.self, from: data)
    }

    /// `at` 에 보일 것 — 끝난 일정·시작한 부탁을 뺀다(피드는 만든 때 기준이라 타임라인의 뒤 칸에선 다시 거른다).
    func items(at now: Date) -> [Item] {
        items.filter { $0.errand ? $0.start >= now : ($0.end > now || $0.start >= now) }
    }

    /// 보이는 것이 바뀌는 때 — 각 일정의 끝·부탁의 시작·자정(«오늘/내일» 이 바뀐다). 타임라인 칸이 된다.
    func changes(after now: Date, limit: Int = 24) -> [Date] {
        let cal = Calendar.current
        var t = Set(items.map { $0.errand ? $0.start.addingTimeInterval(1) : max($0.end, $0.start.addingTimeInterval(1)) })
        // 시작하면 «지금» 으로 바뀐다.
        t.formUnion(items.filter { !$0.errand && !$0.allDay }.map(\.start))
        var midnight = cal.startOfDay(for: now)
        for _ in 0..<7 {
            midnight = cal.date(byAdding: .day, value: 1, to: midnight)!
            t.insert(midnight)
        }
        return Array(t.filter { $0 > now }.sorted().prefix(limit))
    }
}

/// 위젯·앱이 같이 쓰는 이동 주소 — 위젯을 누르면 앱이 그 항목을 연다.
enum OuroLink {
    static func item(_ i: WidgetFeed.Item) -> URL { URL(string: "ouro://\(i.errand ? "errand" : "event")/\(i.id)")! }
    static func errand(_ id: String) -> URL { URL(string: "ouro://errand/\(id)")! }
    static let today = URL(string: "ouro://today")!
}
