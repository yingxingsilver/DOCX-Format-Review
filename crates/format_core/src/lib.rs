//! format_core —— 论文 DOCX 格式审核核心引擎。
//!
//! 设计原则：
//! - 不依赖 Tauri，可独立编译、独立测试、未来可复用于 CLI。
//! - 运行时不使用 LLM，不联网，不依赖 Microsoft Word。
//! - 解析基于 zip + quick-xml（P2 起启用）。
//! - 规则通过 JSON/YAML 配置驱动（P3 起启用）。
//!
//! 模块划分：
//! - [`dsl`]           权威数据模型（文档 DSL + 规则配置结构，与 rules/*.json 一一对应）
//! - [`docx_parser`]   OOXML 解包与 document.xml / styles.xml 解析
//! - [`style_resolver`] 样式继承链解析（直接格式 > 字符样式 > 段落样式 > docDefaults）
//! - [`rules`]         规则配置加载（JSON/YAML）与语义校验
//! - [`audit`]         规则执行引擎（批量处理基于 rayon）
//! - [`report`]        面向非技术人员中文报告生成（占位）
//! - [`error`]         统一错误类型
//!
//! 注：旧 `models` 模块（早期 RuleConfig/DocumentModel 原型）已随审查项 S2 删除，
//! 其职责由 `dsl` 全量承接；单位换算入口见 `dsl::half_points_to_pt`。

pub mod audit;
pub mod checkers;
pub mod docx_parser;
pub mod dsl;
pub mod error;
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
        // P6：report 不再是占位模块，验证真实入口存在。
        let _: fn(
            &dsl::AuditReport,
            &std::path::Path,
        ) -> error::CoreResult<()> = report::generate_html_report;
        let _: fn(
            &dsl::AuditReport,
            &std::path::Path,
        ) -> error::CoreResult<()> = report::generate_csv_report;
        // models 已删除（S2）：用 dsl 与 error 的真实类型验证可访问。
        let _ = dsl::Severity::Error.label_cn();
        let _ = error::CoreError::RuleInvalid {
            reason: "测试".into(),
        };
    }

    /// P5：audit 批量入口必须存在（进度回调 + 取消令牌 + AuditReport）。
    #[test]
    fn audit_engine_entry_points_exist() {
        let _: fn(
            &crate::dsl::ParsedDocument,
            &crate::dsl::RuleConfig,
        ) -> error::CoreResult<Vec<crate::dsl::Violation>> = audit::audit_document_with_config;
        let _: fn(
            &std::path::Path,
            &crate::dsl::RuleConfig,
            (),
            &audit::CancellationToken,
        ) -> Vec<crate::dsl::FileAuditSummary> =
            |p, c, pr, t| audit::audit_directory(p, c, pr, t).unwrap().files;
    }

    /// 审查项 S1：默认规则文件必须能通过**运行时权威模型**（dsl::RuleConfig）加载，
    /// 而不是早期 models::RuleConfig；不断言硬编码规则数量。
    #[test]
    fn shipped_default_rules_load_with_dsl_schema() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rules/default.json");
        let cfg = rules::load_rule_config(path)
            .unwrap_or_else(|e| panic!("rules/default.json 应能通过 dsl 模型加载: {}", e.user_message()));
        assert!(!cfg.rules.is_empty());
        // ID 不得重复（load_rule_config 内部已做 validate，这里再显式检查一次语义）。
        let mut ids: Vec<&str> = cfg.rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "规则 ID 存在重复");
    }

    /// 审查项补测 2：全部随产品发布的规则文件都必须能加载并构建出检查器表，
    /// 且零 InvalidRegex——防止 schema 漂移再次发生。
    #[test]
    fn all_shipped_rule_files_build_checkers() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        for rel in [
            "rules/default.json",
            "resources/rules/education_generic.json",
        ] {
            let path = std::path::Path::new(root).join(rel);
            let cfg = rules::load_rule_config(&path)
                .unwrap_or_else(|e| panic!("{rel} 加载失败: {}", e.user_message()));
            let built = checkers::build_checkers(&cfg)
                .unwrap_or_else(|e| panic!("{rel} 构建检查器失败: {}", e.user_message()));
            assert!(
                !built.is_empty(),
                "{rel} 至少应产出一个可用检查器"
            );
        }
    }
}
