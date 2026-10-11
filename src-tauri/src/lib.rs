//! Tauri 后端入口（骨架）。
//!
//! 职责边界：本层只做「命令暴露 + 参数转发」，
//! 所有 DOCX 解析/审核业务逻辑一律放在 `format_core`，禁止写入前端或本层。

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![commands::core_version, commands::export_report])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
