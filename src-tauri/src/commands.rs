//! Tauri 命令层（骨架）。
//!
//! 职责边界：本层只做「参数校验 + 对 format_core 的薄封装」，
//! 任何审核/报告业务逻辑一律放在 `format_core`，禁止写入本层或前端。

use std::path::Path;

use serde::{Deserialize, Serialize};

/// 自检命令：前端可调用以确认 Rust 核心引擎已正确链接。
#[tauri::command]
pub fn core_version() -> String {
    format_core::version().to_string()
}

/// 报告格式（与前端下拉选项对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Html,
    Csv,
}

/// P6：把批量审核结果（JSON 序列化的 `AuditReport`）导出为 HTML / CSV 文件。
///
/// - `report_json` 来自 `audit_directory` 返回值的序列化结果；
/// - 渲染逻辑全部在 `format_core::report`，本函数不掺入业务；
/// - 错误返回中文用户提示（`user_message`），技术细节仅进日志。
#[tauri::command]
pub async fn export_report(
    report_json: String,
    output_path: String,
    format: ReportFormat,
) -> Result<String, String> {
    let report: format_core::dsl::AuditReport = serde_json::from_str(&report_json)
        .map_err(|e| {
            format!(
                "审核结果数据有误，无法生成报告：{e}\n（请重新运行一次检查后再导出）"
            )
        })?;
    let path = Path::new(&output_path);
    let result = match format {
        ReportFormat::Html => format_core::report::generate_html_report(&report, path),
        ReportFormat::Csv => format_core::report::generate_csv_report(&report, path),
    };
    match result {
        Ok(()) => Ok(format!("报告已保存到：{output_path}")),
        Err(e) => {
            // 日志留全链路，用户只见中文提示。
            eprintln!("[export_report] {}", e.debug_chain());
            Err(e.user_message())
        }
    }
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

    #[test]
    fn report_format_deserializes_snake_case() {
        let h: ReportFormat = serde_json::from_str("\"html\"").unwrap();
        let c: ReportFormat = serde_json::from_str("\"csv\"").unwrap();
        assert_eq!(h, ReportFormat::Html);
        assert_eq!(c, ReportFormat::Csv);
    }

    /// 非法 JSON 必须得到中文、含引导语的错误，而不是 panic。
    #[test]
    fn export_report_rejects_bad_json_with_chinese_hint() {
        // 反序列化在函数体首行完成（同步），可直接用阻塞方式驱动 future 到 Ready。
        let mut fut = std::pin::pin!(export_report(
            "{ 坏数据".into(),
            "/tmp/不应被写入.html".into(),
            ReportFormat::Html,
        ));
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let err = match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(Err(e)) => e,
            _ => panic!("参数错误应同步返回 Err"),
        };
        assert!(err.contains("审核结果数据有误"), "{err}");
        assert!(!std::path::Path::new("/tmp/不应被写入.html").exists());
    }
}
