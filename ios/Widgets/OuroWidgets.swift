// 위젯 확장 — 지금은 컨트롤 하나(개발 14). 제어 센터·잠금 화면·동작 버튼에 놓으면 입력칸이 열린 Ouro 를 연다.
// 다음 일정·답 도착 위젯은 개발 15.

import AppIntents
import SwiftUI
import WidgetKit

struct QuickAddControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "com.dinggi5.ouro.ios.quick-add") {
            ControlWidgetButton(action: OpenQuickAddIntent()) {
                Label("Ouro 빠른 추가", systemImage: "circle.dashed")
            }
        }
        .displayName("빠른 추가")
        .description("입력칸이 열린 Ouro 를 엽니다.")
    }
}

@main
struct OuroWidgets: WidgetBundle {
    var body: some Widget {
        QuickAddControl()
    }
}
