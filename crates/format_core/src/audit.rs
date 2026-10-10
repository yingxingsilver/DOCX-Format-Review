//! audit —— 规则执行引擎：单文档审核入口 + 批量 rayon 并行。
//!
//! 流程：
//! 1. `build_checkers(config)` 构建全部启用规则的检查器（正则已在加载期预编译）。
//! 2. 文档级规则调用 `check_document`；段落级规则按 `TargetSpec` 过滤后调用 `check_paragraph`。
//! 3. 同一 (paragraph_index, rule_id) 的重复问题聚合为一条，避免刷屏。
//! 4. 每条规则用 `catch_unwind` 包裹：单个检查器 panic 只影响该规则，不中断整篇/整批。
//! 5. 最终按 file_path → page_hint → paragraph_index → rule_id 排序输出。

use std::collections::HashMap;
use std::path::Path;

use rayon::prelude::*;
use regex::Regex;

use crate::checkers::{self, CompiledChecker};
use crate::dsl::{
    AuditContext, FileAuditSummary, FileStatus, Paragraph, ParagraphRole, ParsedDocument,
    RuleConfig, Severity, TargetSpec, Violation,
};
use crate::error::{CoreError, CoreResult};

/// 骨架自检占位（保持与 lib.rs 模块链接测试兼容）。
pub fn placeholder_ok() -> bool {
    true
}

// =========================
// TargetSpec 匹配
// =========================

/// target 是否命中该段落（不含 text_regex 判断）。roles 为空表示匹配所有段落。
pub fn target_matches(target: Option<&TargetSpec>, p: &Paragraph) -> bool {
    let Some(t) = target else { return true };
    let role_hit = |name: &str| match ParagraphRole::from_kind_str(name) {
        Some(ParagraphRole::Heading { .. }) => p.role.heading_level().is_some(),
        Some(other) => p.role == other,
        None => false,
    };
    if !t.roles.is_empty() && !t.roles.iter().any(|r| role_hit(r)) {
        return false;
    }
    if let Some(level) = t.level {
        if p.role.heading_level() != Some(level) {
            return false;
        }
    }
    if !t.exclude_roles.is_empty() && t.exclude_roles.iter().any(|r| role_hit(r)) {
        return false;
    }
    if !t.tags.is_empty() {
        let all_present = t.tags.iter().all(|name| {
            crate::dsl::RoleTag::from_str_name(name)
                .map(|tag| p.role_tags.contains(&tag))
                .unwrap_or(false)
        });
        if !all_present {
            return false;
        }
    }
    true
}

/// 对带 text_regex 的目标做正则匹配（正则非法视为不命中；加载期已校验过合法性）。
pub fn matches_text_regex(t: &TargetSpec, p: &Paragraph) -> bool {
    match t.text_regex.as_deref().and_then(|pat| Regex::new(pat).ok()) {
        Some(re) => re.is_match(&p.text),
        None => true,
    }
}

/// 段落是否被某个检查器选中（target roles/level/tags + text_regex 全条件）。
fn checker_selects(c: &CompiledChecker, p: &Paragraph) -> bool {
    target_matches(c.target(), p)
        && match c.target() {
            Some(t) => matches_text_regex(t, p),
            None => true,
        }
}

// =========================
// 单文档审核
// =========================

/// 对单个已解析文档执行全部启用规则，返回去重、排序后的违规列表。
pub fn audit_document_with_config(
    doc: &ParsedDocument,
    config: &RuleConfig,
) -> CoreResult<Vec<Violation>> {
    let checkers = checkers::build_checkers(config)?;
    Ok(run_checkers(doc, config, &checkers))
}

fn run_checkers(
    doc: &ParsedDocument,
    config: &RuleConfig,
    checkers: &[CompiledChecker],
) -> Vec<Violation> {
    let mut out: Vec<Violation> = Vec::new();
    for c in checkers {
        let ctx = AuditContext { document: doc, config, rule: &c.rule };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if c.is_document_level() {
                c.check_document(&ctx)
            } else {
                doc.paragraphs
                    .iter()
                    .filter(|p| checker_selects(c, p))
                    .flat_map(|p| c.check_paragraph(&ctx, p))
                    .collect::<Vec<_>>()
            }
        }));
        match result {
            Ok(vs) => out.extend(vs),
            Err(_) => {
                // 检查器内部缺陷：降级为提示，建议人工核对；不影响其余规则与文件。
                out.push(crate::checkers::violation_for(
                    &c.rule,
                    &doc.path,
                    Default::default(),
                    format!(
                        "“{}”这一项本次未能完成检查，建议人工核对。",
                        c.rule.message.trim_end_matches('。')
                    ),
                    None,
                    Some("未能完成检查".into()),
                    None,
                    0.0,
                ));
            }
        }
    }
    dedupe_by_paragraph_rule(out)
}

/// 同一 (paragraph_index, rule_id) 只保留最严重的一条，避免一个段落刷屏。
fn dedupe_by_paragraph_rule(vs: Vec<Violation>) -> Vec<Violation> {
    let mut best: HashMap<(Option<usize>, String), usize> = HashMap::new();
    let mut kept: Vec<Violation> = Vec::new();
    for v in vs {
        let key = (v.location.paragraph_index, v.rule_id.clone());
        match best.get(&key) {
            Some(&idx) => {
                if v.severity < kept[idx].severity {
                    kept[idx] = v;
                }
            }
            None => {
                best.insert(key, kept.len());
                kept.push(v);
            }
        }
    }
    sort_violations(&mut kept);
    kept
}

/// 稳定排序：file_path → page_hint（None 最后）→ paragraph_index → rule_id。
pub fn sort_violations(vs: &mut [Violation]) {
    vs.sort_by(|a, b| {
        let page_ord = a
            .location
            .page_hint
            .unwrap_or(u32::MAX)
            .cmp(&b.location.page_hint.unwrap_or(u32::MAX));
        let para_ord = a
            .location
            .paragraph_index
            .unwrap_or(usize::MAX)
            .cmp(&b.location.paragraph_index.unwrap_or(usize::MAX));
        a.file_path
            .cmp(&b.file_path)
            .then(page_ord)
            .then(para_ord)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
    });
}

// =========================
// 文件级容错与批量审核
// =========================

/// 把底层错误转成面向普通用户的中文说明（绝不外露 XML/路径/堆栈细节）。
pub fn friendly_read_error(err: &CoreError) -> String {
    err.user_message()
}

fn read_failed(path: &Path, message: String) -> FileAuditSummary {
    FileAuditSummary {
        file_path: path.to_path_buf(),
        file_name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        status: FileStatus::ReadFailed,
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        violations: vec![],
        parse_error: Some(message),
    }
}

/// 单个文件的完整审核结果（读取失败/损坏/加密均返回 ReadFailed + 友好文案，不抛异常）。
pub fn audit_file(path: &Path, config: &RuleConfig) -> FileAuditSummary {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::docx_parser::parse_docx(path)
    }));
    let doc = match parsed {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => return read_failed(path, friendly_read_error(&e)),
        Err(_) => return read_failed(path, "这份文件无法读取，可能已损坏或受密码保护。".into()),
    };
    let checkers = match checkers::build_checkers(config) {
        Ok(c) => c,
        Err(e) => return read_failed(path, friendly_read_error(&e)),
    };
    let violations = run_checkers(&doc, config, &checkers);
    summarize(doc.path.clone(), doc.file_name.clone(), violations, None)
}

fn summarize(
    file_path: std::path::PathBuf,
    file_name: String,
    violations: Vec<Violation>,
    parse_error: Option<String>,
) -> FileAuditSummary {
    let error_count = violations.iter().filter(|v| v.severity == Severity::Error).count();
    let warning_count = violations.iter().filter(|v| v.severity == Severity::Warning).count();
    let info_count = violations.iter().filter(|v| v.severity == Severity::Info).count();
    let status = if error_count + warning_count > 0 {
        FileStatus::NeedsRevision
    } else {
        FileStatus::Passed
    };
    FileAuditSummary {
        file_path,
        file_name,
        status,
        error_count,
        warning_count,
        info_count,
        violations,
        parse_error,
    }
}

/// 批量审核目录下全部 .docx（rayon 并行；单文件失败不影响其他文件）。
pub fn audit_directory(dir: &Path, config: &RuleConfig) -> CoreResult<Vec<FileAuditSummary>> {
    if !dir.is_dir() {
        return Err(CoreError::InvalidPath {
            path: dir.display().to_string(),
            reason: "所选位置不是一个文件夹".into(),
        });
    }
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_docx(dir, &mut files)?;
    files.sort();
    let summaries = files.par_iter().map(|f| audit_file(f, config)).collect::<Vec<_>>();
    Ok(summaries)
}

fn collect_docx(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> CoreResult<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| CoreError::Io {
        context: "打开论文文件夹".into(),
        source: e,
    })?;
    for entry in entries {
        let entry = entry.map_err(|e| CoreError::Io {
            context: "读取论文文件夹".into(),
            source: e,
        })?;
        let path = entry.path();
        let is_lock_file = path
            .file_name()
            .map(|n| n.to_string_lossy().starts_with("~$"))
            .unwrap_or(false);
        if path.is_dir() {
            collect_docx(&path, out)?;
        } else if !is_lock_file
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("docx"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }
    Ok(())
}

// =========================
// 单元测试
// =========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::font::tests::{empty_doc, para, run_with};
    use crate::dsl::{EffectiveRunFormat, Location};
    use crate::rules::tests_helper::config_from_json;

    fn shipped_cfg() -> RuleConfig {
        config_from_json(include_str!("../../../resources/rules/education_generic.json"))
    }

    fn body_doc(size: Option<f64>) -> ParsedDocument {
        let mut doc = empty_doc();
        let mut r = run_with("教育公平研究的重要性体现在多个层面。", 0, size, None, None);
        r.effective = Some(EffectiveRunFormat { font_size_pt: size, ..Default::default() });
        doc.paragraphs = vec![para(vec![r], ParagraphRole::Body)];
        doc
    }

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }

    #[test]
    fn end_to_end_audit_finds_body_font_size_and_sorts() {
        let cfg = shipped_cfg();
        let vs = audit_document_with_config(&body_doc(Some(10.5)), &cfg).unwrap();
        assert!(vs
            .iter()
            .any(|v| v.rule_id == "body-font-size" && v.severity == Severity::Error));
        // 排序：同文件内按 page_hint → paragraph_index 有序
        let ok = vs.windows(2).all(|w| {
            (
                w[0].location.page_hint.unwrap_or(u32::MAX),
                w[0].location.paragraph_index.unwrap_or(usize::MAX),
            ) <= (
                w[1].location.page_hint.unwrap_or(u32::MAX),
                w[1].location.paragraph_index.unwrap_or(usize::MAX),
            )
        });
        assert!(ok, "违规列表必须按页码/段落排序");
    }

    #[test]
    fn same_paragraph_and_rule_aggregates_to_one_keeping_worst() {
        let mk = |sev: Severity| Violation {
            rule_id: "x".into(),
            severity: sev,
            file_path: "/t.docx".into(),
            location: Location { paragraph_index: Some(1), ..Default::default() },
            message: "m".into(),
            expected: None,
            actual: None,
            excerpt: None,
            suggestion: None,
            confidence: 1.0,
            requires_manual_review: false,
        };
        let out = dedupe_by_paragraph_rule(vec![
            mk(Severity::Warning),
            mk(Severity::Error),
            mk(Severity::Info),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity, Severity::Error, "同段同规则应保留最严重一条");
    }

    #[test]
    fn target_spec_matching_rules() {
        let mut p = para(
            vec![run_with("一、绪论", 0, Some(16.0), None, None)],
            ParagraphRole::Heading { level: 1 },
        );
        let t = |roles: &[&str], level: Option<u8>| TargetSpec {
            roles: roles.iter().map(|s| s.to_string()).collect(),
            level,
            exclude_roles: vec![],
            tags: vec![],
            text_regex: None,
        };
        assert!(target_matches(Some(&t(&["heading"], None)), &p));
        assert!(target_matches(Some(&t(&["heading"], Some(1))), &p));
        assert!(!target_matches(Some(&t(&["heading"], Some(2))), &p));
        assert!(!target_matches(Some(&t(&["body"], None)), &p));
        assert!(target_matches(Some(&t(&[], None)), &p), "空 roles 匹配所有段落");
        let mut excl = t(&[], None);
        excl.exclude_roles = vec!["heading".into()];
        assert!(!target_matches(Some(&excl), &p));
        // text_regex 过滤
        let t2 = TargetSpec {
            roles: vec!["heading".into()],
            level: None,
            exclude_roles: vec![],
            tags: vec![],
            text_regex: Some("^二、".into()),
        };
        assert!(!matches_text_regex(&t2, &p));
        p.text = "二、文献综述".into();
        assert!(matches_text_regex(&t2, &p));
    }

    struct Boom;
    impl crate::dsl::Checker for Boom {
        fn rule_id(&self) -> &str {
            "boom"
        }
        fn check_document(&self, _ctx: &AuditContext<'_>) -> Vec<Violation> {
            panic!("模拟检查器内部缺陷")
        }
    }

    #[test]
    fn panic_in_checker_degrades_not_crashes_batch() {
        let cfg = shipped_cfg();
        let rule = cfg.rules.iter().find(|r| r.id == "page-a4").unwrap();
        let compiled = CompiledChecker::for_test(rule.clone(), Box::new(Boom));
        let doc = empty_doc();
        let vs = run_checkers(&doc, &cfg, std::slice::from_ref(&compiled));
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("人工核对"));
        // 用户文案禁止出现技术术语
        let lower = vs[0].message.to_ascii_lowercase();
        for banned in ["panic", "unwrap", "xml", "w:", "stack"] {
            assert!(!lower.contains(banned));
        }
    }

    #[test]
    fn unreadable_file_yields_read_failed_without_panic() {
        let cfg = shipped_cfg();
        let summary = audit_file(Path::new("/nonexistent-missing-dir/缺失文件.docx"), &cfg);
        assert_eq!(summary.status, FileStatus::ReadFailed);
        let msg = summary.parse_error.clone().unwrap_or_default();
        assert!(!msg.is_empty(), "读取失败必须有中文说明");
        for banned in ["xml", "w:", "panic", "unwrap", "zip", "os error", "stack"] {
            assert!(!msg.to_ascii_lowercase().contains(banned), "外露术语: {msg}");
        }
    }

    #[test]
    fn audit_directory_skips_word_lock_files() {
        let dir = std::env::temp_dir().join("fmt_audit_test_dir");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("~$临时锁.docx"), b"x").unwrap();
        let cfg = shipped_cfg();
        let summaries = audit_directory(&dir, &cfg).unwrap();
        assert!(summaries.is_empty(), "Word 临时锁文件不应进入审核");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
