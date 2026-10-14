// 发布构建（非 tauri dev）在 Windows 上不显示控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    paper_format_auditor_lib::run()
}
