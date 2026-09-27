// 릴리스에서 Windows 콘솔 창을 띄우지 않는다(Tauri 템플릿). macOS 전용이지만 지워도 얻는 게 없다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ouro_lib::run()
}
