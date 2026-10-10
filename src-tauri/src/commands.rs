//! Tauri 命令层（骨架）。
//!
//! P4 起将新增：`audit_files`（批量审核）、`load_rules`、`export_report` 等命令，
//! 全部为对 `format_core` 的薄封装。

/// 自检命令：前端可调用以确认 Rust 核心引擎已正确链接。
#[tauri::command]
pub fn core_version() -> String {
    format_core::version().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_version_returns_semver() {
        let v = core_version();
        // 骨架阶段仅要求形如 x.y.z 且非空
        assert!(v.chars().next().is_some_and(|c| c.is_ascii_digit()));
        assert_eq!(v, "0.1.0");
    }
}
