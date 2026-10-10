fn main() {
    // Tauri 应用在 Windows 上需要 release 构建禁用控制台窗口的指令；
    // 此处保持与 tauri create-app 模板一致。
    paper_format_auditor_lib::run()
}
