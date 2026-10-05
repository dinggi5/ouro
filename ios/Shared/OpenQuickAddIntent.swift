// «빠른 추가 열기» — 컨트롤 버튼(제어 센터·잠금 화면·동작 버튼)이 누르는 인텐트(개발 14).
// 앱과 위젯 확장 **둘 다**에 들어간다: 확장은 버튼에 이 인텐트를 걸기만 하고, 실행(perform)은 앱이 열리며 앱 쪽에서 한다.
// 앱 쪽에서만 `OURO_APP` 이 켜져 있어 입력칸을 연다.

import AppIntents

struct OpenQuickAddIntent: AppIntent {
    static let title: LocalizedStringResource = "빠른 추가 열기"
    static let description = IntentDescription("입력칸이 열린 Ouro 를 엽니다. «내일 3시 치과» 처럼 쓰면 돼요.")
    static let supportedModes: IntentModes = .foreground(.immediate)

    @MainActor
    func perform() async throws -> some IntentResult {
        #if OURO_APP
            AppModel.shared.request = .quickAdd
        #endif
        return .result()
    }
}
