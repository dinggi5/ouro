// `-demo` 로 켜면 메모리에만 있는 예시 하루(스크린숏·화면 확인용). 동기화는 안 켠다.

import Foundation
import OuroSyncKit

@MainActor
enum Demo {
    static func fill(_ s: RecordStore) {
        let cal = Calendar.current
        let today = cal.startOfDay(for: Date())
        func at(_ h: Int, _ m: Int = 0) -> Date { cal.date(bySettingHour: h, minute: m, second: 0, of: today)! }
        var e = EventDraft()
        e.title = "치과"
        e.start = at(15); e.end = at(16)
        try? s.createEvent(e)
        e.title = "팀 회의"
        e.start = at(11); e.end = at(12)
        try? s.createEvent(e)
        e.title = "엄마 생신"
        e.allDay = true
        e.start = today; e.end = today
        try? s.createEvent(e)
        var r = ErrandDraft()
        r.prompt = "어제 커밋 정리해 줘"
        r.at = at(9)
        let done = (try? s.createErrand(r)) ?? ""
        r.prompt = "오늘 회의 안건 초안"
        r.at = at(18)
        try? s.createErrand(r)
        let now = Clock.ms()
        try? s.apply([
            WireRecord(type: Kind.run, name: "demo-run", bag: [
                "item": .string(done), "status": .string("done"), "started_at": .int(at(9).ms), "finished_at": .int(at(9, 2).ms),
                "response": .string("어제는 동기화 코어(CKSyncEngine 헬퍼)와 실행 맥 지정을 넣었어요. 리뷰 지적 15건을 반영했고, 테스트는 135개가 통과해요."),
                "sent_text": .string("어제 커밋 정리해 줘"),
            ], system: nil),
            WireRecord(type: Kind.proposal, name: "demo-proposal", bag: [
                "client": .string("claude-code"), "title": .string("금요일 치과 재진"), "notes": .string(""), "all_day": .int(0),
                "start_at": .int(cal.date(byAdding: .day, value: 4, to: at(15))!.ms),
                "end_at": .int(cal.date(byAdding: .day, value: 4, to: at(16))!.ms),
                "status": .string("pending"), "created_at": .int(now),
            ], system: nil),
        ], deleted: [])
    }
}
