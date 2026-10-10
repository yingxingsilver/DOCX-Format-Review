//! models —— 审核结果数据模型（占位）。
//!
//! P1/P2 将定义：`DocumentModel`、`ParagraphInfo`、`RunInfo`、
//! `AuditIssue`、`Severity`、`AuditReport` 等结构与枚举。

/// 骨架占位，供 lib.rs 的模块链接测试使用。
pub fn placeholder_ok() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }
}
