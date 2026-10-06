// 위젯 피드 쓰기(개발 15) — 저장소가 디스크에 쓸 때마다 잠깐 모았다가 피드를 다시 만들고, 바뀌었을 때만 위젯을 새로 그리게 한다
// (위젯 새로 그리기는 하루 횟수 제한이 있다 — 받기마다 부르면 정작 필요할 때 막힌다).

import Foundation
import OuroSyncKit
import WidgetKit

@MainActor
final class WidgetWriter {
    private let store: RecordStore
    private var pending: Task<Void, Never>?
    private var last: WidgetFeed?

    init(store: RecordStore) {
        self.store = store
        last = WidgetFeed.load()
    }

    /// 저장소가 바뀌었다 — 0.5초 모아서 한 번.
    func changed() {
        pending?.cancel()
        pending = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            self?.write()
        }
    }

    /// 지금 만든다(앱이 뒤로 갈 때 — 모으는 중이던 것이 잠들기 전에 나가게).
    func write(now: Date = Date()) {
        pending?.cancel()
        pending = nil
        let feed = Self.build(store, now: now)
        guard feed != last, let url = WidgetFeed.url else { return }
        do {
            // 잠긴 폰에서도 첫 잠금 해제 뒤면 위젯이 읽는다(저장소 파일과 같은 보호).
            try JSONEncoder().encode(feed).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            last = feed
            WidgetCenter.shared.reloadTimelines(ofKind: WidgetFeed.kind)
            WidgetCenter.shared.reloadTimelines(ofKind: WidgetFeed.answersKind)
        } catch {
            // 위젯은 덤이다 — 못 써도 앱은 그대로. 다음 변경 때 다시 쓴다.
        }
    }

    static func build(_ store: RecordStore, now: Date) -> WidgetFeed {
        let answers = store.unreadAnswers()
        return WidgetFeed(
            items: store.upcoming(from: now).map {
                .init(id: $0.id, title: $0.title, allDay: $0.allDay, start: $0.start, end: $0.end, errand: $0.errand)
            },
            answers: answers.prefix(3).map { .init(errand: $0.errand, title: $0.title, at: $0.finishedAt) },
            unread: answers.count)
    }
}
