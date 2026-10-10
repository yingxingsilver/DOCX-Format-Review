//! format_core —— 论文 DOCX 格式审核核心引擎。
//!
//! 设计原则：
//! - 不依赖 Tauri，可独立编译、独立测试、未来可复用于 CLI。
//! - 运行时不使用 LLM，不联网，不依赖 Microsoft Word。
//! - 解析基于 zip + quick-xml（P1 起启用）。
//! - 规则通过 JSON/YAML 配置驱动（P3 起启用）。
//!
//! 模块划分（骨架阶段均为占位，不含业务逻辑）：
//! - [`models`]        审核结果的数据模型（问题、严重级别、文档结构等）
//! - [`docx_parser`]   OOXML 解包与 document.xml / styles.xml 解析
//! - [`style_resolver`] 样式继承链解析（直接格式 > 字符样式 > 段落样式 > docDefaults）
//! - [`rules`]         规则配置加载（JSON/YAML）与规则定义
//! - [`audit`]         规则执行引擎（批量处理基于 rayon）
//! - [`report`]        面向非技术人员的中文报告生成
//! - [`error`]         统一错误类型

pub mod audit;
pub mod docx_parser;
pub mod error;
pub mod models;
pub mod report;
pub mod rules;
pub mod style_resolver;

/// 骨架自检：确认核心库可被链接与调用（不含业务逻辑）。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_not_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn all_modules_are_linked() {
        // 仅验证各模块均可访问，防止模块遗漏导出。
        assert!(docx_parser::placeholder_ok());
        assert!(style_resolver::placeholder_ok());
        assert!(rules::placeholder_ok());
        assert!(audit::placeholder_ok());
        assert!(report::placeholder_ok());
        // P1：models 与 error 已有真实类型，用构造调用验证可访问。
        let _ = models::Severity::Error.label_cn();
        let _ = error::CoreError::rule_invalid("测试");
    }

    /// P1 验收：仓库内示例规则文件必须能被 RuleConfig 数据模型完整解析。
    #[test]
    fn shipped_default_rules_match_schema() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rules/default.json");
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("无法读取 {path}: {e}"));
        let cfg: models::RuleConfig = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("rules/default.json 不符合 RuleConfig 结构: {e}"));
        assert_eq!(cfg.rules.len(), 9);
        // ID 不得重复。
        let mut ids: Vec<&str> = cfg.rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "规则 ID 存在重复");
    }
}
