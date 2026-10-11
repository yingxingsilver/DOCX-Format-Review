//! Tauri 后端入口（P8D GUI 整合）。
//!
//! 职责边界：本层只做「命令暴露 + 状态管理 + 生命周期钩子」，
//! 所有 DOCX 解析/审核业务逻辑一律放在 `format_core`，禁止写入前端或本层。

mod commands;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(commands::AuditJobs::default())
        .invoke_handler(tauri::generate_handler![
            commands::core_version,
            commands::list_docx_files,
            commands::validate_rule_file,
            commands::start_audit,
            commands::cancel_audit,
            commands::export_report
        ])
        .on_window_event(|window, event| {
            // 主窗口关闭时取消全部在途审核任务，避免后台僵尸线程继续跑批。
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    let jobs = window.state::<commands::AuditJobs>();
                    let n = jobs.cancel_all();
                    if n > 0 {
                        eprintln!("[lifecycle] 窗口关闭，已请求取消 {n} 个在途审核任务。");
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
