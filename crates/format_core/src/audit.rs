//! audit —— 规则执行引擎：单文档审核入口 + P5 批量调度（rayon 并行、进度回调、取消）。
//!
//! 流程：
//! 1. `build_checkers(config)` 构建全部启用规则的检查器（正则已在加载/构建期编译一次）。
//! 2. 文档级规则调用 `check_document`；段落级规则按 `TargetSpec` 过滤后调用 `check_paragraph`。
//! 3. 同一 (paragraph_index, rule_id) 的重复问题聚合为一条，避免刷屏；文档级违规不参与折叠。
//! 4. 每条规则用 `catch_unwind` 包裹：单个检查器 panic 只影响该规则，不中断整篇/整批。
//! 5. 最终按 file_path → page_hint → paragraph_index → rule_id 排序输出。
//!
//! P5 批量调度约定：
//! - 核心层保持同步 API（rayon，不引入 tokio）；上层（Tauri）负责 spawn_blocking 与事件转发。
//! - 任何内部错误一律经 [`AuditProgress::on_log`] / 报告字段返回，绝不 `println!`。
//! - 用户可见文案中文、友好，不含 XML 节点名 / 堆栈 / 内部路径细节。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rayon::prelude::*;

use crate::checkers::{self, CompiledChecker};
use crate::dsl::{
    AuditContext, AuditReport, FileAuditSummary, FileStatus, Paragraph, ParagraphRole,
    ParsedDocument, RuleConfig, Severity, TargetSpec, Violation,
};
use crate::error::{CoreError, CoreResult};

/// 骨架自检占位（保持与 lib.rs 模块链接测试兼容）。
pub fn placeholder_ok() -> bool {
    true
}

// =========================
// 进度回调与取消令牌
// =========================

/// 批量审核进度回调。实现方必须快速返回（例如向 GUI 发事件），不要做长耗时操作。
/// 默认实现全部为空操作，方便测试与“只要结果不要进度”的调用方。
pub trait AuditProgress {
    /// 某个文件开始处理（在工作线程上被调用，实现需可并发安全）。
    fn on_file_started(&self, _file: &Path) {}
    /// 某个文件处理完成；`violation_count` 为该文件的违规条数（读取失败时为 0）。
    fn on_file_finished(&self, _file: &Path, _violation_count: usize) {}
    /// 面向用户的日志行（中文）。核心层所有“打印”都走这里。
    fn on_log(&self, _message: String) {}
}

impl AuditProgress for () {}

/// 轻量取消令牌：`AtomicBool`，跨线程共享用 `Arc`。已取消的任务不再启动新文件，
/// 正在处理的当前文件会自然跑完（不做强杀，保证内存安全）。
#[derive(Debug, Default)]
pub struct CancellationToken {
    cancelled: AtomicBool,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    /// 请求取消。可多次调用，幂等。
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
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

/// 对带 text_regex 的目标做正则匹配（正则非法视为不命中；构建期已编译一次）。
pub fn matches_text_regex(t: &TargetSpec, p: &Paragraph) -> bool {
    match t.text_regex.as_deref().and_then(|pat| regex::Regex::new(pat).ok()) {
        Some(re) => re.is_match(&p.text),
        None => true,
    }
}

/// 段落是否被某个检查器选中（target roles/level/tags + 预编译 text_regex 全条件）。
/// 热路径使用 CompiledChecker 内缓存的正则，避免逐段重编译（性能/DoS 修复项）。
fn checker_selects(c: &CompiledChecker, p: &Paragraph) -> bool {
    target_matches(c.target(), p)
        && c.compiled_text_regex().map_or(true, |re| re.is_match(&p.text))
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
/// 文档级违规（paragraph_index 为 None）不折叠——多节页面设置问题各自独立成条。
fn dedupe_by_paragraph_rule(vs: Vec<Violation>) -> Vec<Violation> {
    let mut best: HashMap<(usize, String), usize> = HashMap::new();
    let mut kept: Vec<Violation> = Vec::new();
    for v in vs {
        let Some(pi) = v.location.paragraph_index else {
            kept.push(v);
            continue;
        };
        let key = (pi, v.rule_id.clone());
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
// 文件级容错
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
    let shared = checkers::build_checkers(config)
        .map(Arc::new)
        .unwrap_or_else(|_| Arc::new(Vec::new()));
    audit_file_with(path, config, &shared)
}

/// 复用已构建检查器的单文件审核（批量场景避免每文件重建正则表）。
/// `checkers` 为空视为配置错误 → ReadFailed 友好文案。
fn audit_file_with(
    path: &Path,
    config: &RuleConfig,
    checkers: &Arc<Vec<CompiledChecker>>,
) -> FileAuditSummary {
    if checkers.is_empty() {
        return read_failed(path, "规则配置中没有可用的检查项，请先检查规则文件。".into());
    }
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::docx_parser::parse_docx(path)
    }));
    let doc = match parsed {
        Ok(Ok(doc)) => doc,
        Ok(Err(e)) => return read_failed(path, friendly_read_error(&e)),
        Err(_) => return read_failed(path, "这份文件无法读取，可能已损坏或受密码保护。".into()),
    };
    let violations = run_checkers(&doc, config, checkers);
    summarize(doc.path.clone(), doc.file_name.clone(), violations, None)
}

fn summarize(
    file_path: PathBuf,
    file_name: String,
    violations: Vec<Violation>,
    parse_error: Option<String>,
) -> FileAuditSummary {
    let error_count = violations.iter().filter(|v| v.severity == Severity::Error).count();
    let warning_count = violations.iter().filter(|v| v.severity == Severity::Warning).count();
    let info_count = violations.iter().filter(|v| v.severity == Severity::Info).count();
    // “需要修改”只由确定的问题驱动：人工复核类（manual_review）提示不判负，
    // 避免信息不全的最小文档被误标为不合格。
    let confirmed = violations
        .iter()
        .filter(|v| !v.requires_manual_review)
        .filter(|v| matches!(v.severity, Severity::Error | Severity::Warning))
        .count();
    let status = if confirmed > 0 {
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

// =========================
// P5 批量审核调度
// =========================

/// 递归收集目录下全部 .docx（跳过 Word 临时锁文件 `~$xxx.docx`），按路径排序保证结果稳定。
pub fn collect_docx_files(dir: &Path) -> CoreResult<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_docx_into(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_docx_into(dir: &Path, out: &mut Vec<PathBuf>) -> CoreResult<()> {
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
            collect_docx_into(&path, out)?;
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

/// 便捷入口：递归审核目录下全部 .docx，支持进度回调与取消，返回汇总 [`AuditReport`]。
///
/// - rayon 并行；单个文件失败（损坏/加密/空文件/内部 panic）记为 ReadFailed，不中断整体。
/// - 取消：已取消后不再启动新文件，剩余文件标记 Skipped；报告 `rule_profile` 之外字段照常汇总。
/// - 全程无 `println!`；一切消息通过 `progress.on_log` 上报（中文）。
pub fn audit_directory(
    dir: impl AsRef<Path>,
    config: &RuleConfig,
    progress: impl AuditProgress + Sync,
    token: &CancellationToken,
) -> CoreResult<AuditReport> {
    let dir = dir.as_ref();
    if !dir.is_dir() {
        return Err(CoreError::InvalidPath {
            path: dir.display().to_string(),
            reason: "所选位置不是一个文件夹".into(),
        });
    }
    progress.on_log(format!("开始扫描文件夹：{}", dir.display()));
    let files = collect_docx_files(dir)?;
    let total_files = files.len();
    if total_files == 0 {
        progress.on_log("这个文件夹里没有找到 Word 论文文件（.docx）。".into());
    } else {
        progress.on_log(format!("共找到 {total_files} 个论文文件，开始并行检查。"));
    }

    // 检查器表构建一次，Arc 共享给所有 rayon 任务（避免每文件重建正则，审查项 13）。
    let shared: Arc<Vec<CompiledChecker>> = Arc::new(checkers::build_checkers(config)?);

    let summaries: Vec<FileAuditSummary> = files
        .par_iter()
        .map(|f| {
            if token.is_cancelled() {
                return FileAuditSummary {
                    file_path: f.clone(),
                    file_name: f.file_name().unwrap_or_default().to_string_lossy().to_string(),
                    status: FileStatus::Skipped,
                    error_count: 0,
                    warning_count: 0,
                    info_count: 0,
                    violations: vec![],
                    parse_error: Some("任务已取消，此文件未检查。".into()),
                };
            }
            progress.on_file_started(f);
            let s = audit_file_with(f, config, &shared);
            progress.on_file_finished(f, s.violations.len());
            if s.status == FileStatus::ReadFailed {
                progress.on_log(format!(
                    "{}：{}",
                    s.file_name,
                    s.parse_error.clone().unwrap_or_else(|| "无法读取。".into())
                ));
            }
            s
        })
        .collect();

    let mut summaries = summaries;
    summaries.sort_by(|a, b| a.file_name.cmp(&b.file_name).then_with(|| a.file_path.cmp(&b.file_path)));

    let passed_files = summaries
        .iter()
        .filter(|s| s.status == FileStatus::Passed || s.status == FileStatus::NeedsRevision)
        .count();
    let read_failed_files = summaries.iter().filter(|s| s.status == FileStatus::ReadFailed).count();
    let skipped_files = summaries.iter().filter(|s| s.status == FileStatus::Skipped).count();
    let failed_files = summaries.iter().filter(|s| s.status == FileStatus::NeedsRevision).count();
    let total_errors = summaries.iter().map(|s| s.error_count).sum();
    let total_warnings = summaries.iter().map(|s| s.warning_count).sum();
    let total_infos = summaries.iter().map(|s| s.info_count).sum();
    let mut all_violations: Vec<Violation> = summaries.iter().flat_map(|s| s.violations.clone()).collect();
    sort_violations(&mut all_violations);

    if token.is_cancelled() {
        progress.on_log(format!(
            "任务已取消：{skipped_files} 个文件未检查，已完成的部分结果仍可查看。"
        ));
    } else {
        progress.on_log(format!(
            "检查完成：共 {total_files} 个文件，{failed_files} 个需要修改，{read_failed_files} 个无法读取。"
        ));
    }

    Ok(AuditReport {
        generated_at: now_display(),
        input_dir: dir.to_path_buf(),
        rule_file: PathBuf::from(&config.name),
        rule_profile: None,
        total_files,
        passed_files,
        failed_files,
        read_failed_files,
        total_errors,
        total_warnings,
        total_infos,
        files: summaries,
    })
}

/// 旧签名兼容入口（无进度/不可取消）：内部委托新版并丢弃回调。
pub fn audit_directory_summaries(dir: &Path, config: &RuleConfig) -> CoreResult<Vec<FileAuditSummary>> {
    let report = audit_directory(dir, config, (), &CancellationToken::new())?;
    Ok(report.files)
}

/// 时间戳：优先 chrono；为避免为此新增依赖，退化到 std::time 的 UTC 秒级格式。
fn now_display() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // civil-from-days（Howard Hinnant 算法）换算 UTC 日期时间，纯 std 无依赖。
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097).max(0);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
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
    use std::sync::Mutex;

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
    fn document_level_violations_are_not_folded() {
        // 审查项：paragraph_index=None 的多节页面问题不应被去重折叠成一条。
        let mk = |i: u32| Violation {
            rule_id: "page-a4".into(),
            severity: Severity::Error,
            file_path: "/t.docx".into(),
            location: Location { paragraph_index: None, page_hint: Some(i), ..Default::default() },
            message: "页面尺寸不符".into(),
            expected: None,
            actual: None,
            excerpt: None,
            suggestion: None,
            confidence: 1.0,
            requires_manual_review: false,
        };
        let out = dedupe_by_paragraph_rule(vec![mk(1), mk(2), mk(3)]);
        assert_eq!(out.len(), 3, "文档级违规必须逐条保留");
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
        let dir = unique_tmp_dir("lock");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("~$临时锁.docx"), b"x").unwrap();
        let cfg = shipped_cfg();
        let report = audit_directory(&dir, &cfg, (), &CancellationToken::new()).unwrap();
        assert_eq!(report.total_files, 0, "Word 临时锁文件不应进入审核");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------------- P5 批量调度测试 ----------------

    /// 每个测试独立临时目录，避免 rayon 全局池/并行测试互相污染。
    fn unique_tmp_dir(tag: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("fmt_p5_{tag}_{}_{}", std::process::id(), n))
    }

    /// 写一个最小合法 zip（PK\x03\x04 头 + 中央目录），sidecar JSON 提供文档内容。
    fn write_fake_docx(dir: &Path, name: &str, size: Option<f64>) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, [b'P', b'K', 3, 4, 0, 0, 0, 0]).unwrap();
        let mut doc = body_doc(size);
        doc.path = path.clone();
        doc.file_name = name.to_string();
        std::fs::write(
            dir.join(format!("{name}.parsed.json")),
            serde_json::to_string(&doc).unwrap(),
        )
        .unwrap();
        path
    }

    #[derive(Default)]
    struct Recorder {
        started: Mutex<Vec<String>>,
        finished: Mutex<Vec<(String, usize)>>,
        logs: Mutex<Vec<String>>,
    }
    impl AuditProgress for Recorder {
        fn on_file_started(&self, file: &Path) {
            self.started.lock().unwrap().push(file.display().to_string());
        }
        fn on_file_finished(&self, file: &Path, violation_count: usize) {
            self.finished.lock().unwrap().push((file.display().to_string(), violation_count));
        }
        fn on_log(&self, message: String) {
            self.logs.lock().unwrap().push(message);
        }
    }

    #[test]
    fn batch_three_files_one_corrupted_records_failure_and_continues() {
        let dir = unique_tmp_dir("batch");
        std::fs::create_dir_all(&dir).unwrap();
        write_fake_docx(&dir, "甲.docx", Some(12.0));
        write_fake_docx(&dir, "乙.docx", Some(10.5)); // 字号违规
        std::fs::write(dir.join("丙.docx"), b"this is not a docx at all").unwrap(); // 损坏
        let cfg = shipped_cfg();
        let rec = Arc::new(Recorder::default());
        struct Proxy(Arc<Recorder>);
        impl AuditProgress for Proxy {
            fn on_file_started(&self, f: &Path) { self.0.on_file_started(f) }
            fn on_file_finished(&self, f: &Path, n: usize) { self.0.on_file_finished(f, n) }
            fn on_log(&self, m: String) { self.0.on_log(m) }
        }
        let report = audit_directory(&dir, &cfg, Proxy(rec.clone()), &CancellationToken::new()).unwrap();

        assert_eq!(report.total_files, 3);
        assert_eq!(report.read_failed_files, 1, "损坏文件必须记为读取失败");
        let broken = report.files.iter().find(|f| f.status == FileStatus::ReadFailed).unwrap();
        assert_eq!(broken.file_name, "丙.docx");
        let msg = broken.parse_error.clone().unwrap_or_default();
        assert!(msg.contains("损坏") || msg.contains("密码"), "{msg}");
        for banned in ["xml", "zip", "panic", "unwrap", "stack", "w:"] {
            assert!(!msg.to_ascii_lowercase().contains(banned), "外露术语: {msg}");
        }
        // 其他文件照常审核：乙有字号违规，甲通过。
        let yi = report.files.iter().find(|f| f.file_name == "乙.docx").unwrap();
        assert_eq!(yi.status, FileStatus::NeedsRevision);
        assert!(yi.violations.iter().any(|v| v.rule_id == "body-font-size"));
        let jia = report.files.iter().find(|f| f.file_name == "甲.docx").unwrap();
        assert_eq!(jia.status, FileStatus::Passed);
        // 结果按文件名排序稳定。
        let names: Vec<&str> = report.files.iter().map(|f| f.file_name.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "报告必须按文件名排序");
        // 进度回调：3 个文件全部 started + finished。
        assert_eq!(rec.started.lock().unwrap().len(), 3);
        assert_eq!(rec.finished.lock().unwrap().len(), 3);
        assert!(rec.logs.lock().unwrap().iter().any(|l| l.contains("检查完成")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_stops_remaining_files() {
        let dir = unique_tmp_dir("cancel");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..8 {
            write_fake_docx(&dir, &format!("文件{i}.docx"), Some(12.0));
        }
        let cfg = shipped_cfg();
        let token = CancellationToken::new();
        token.cancel(); // 开始前就取消：不启动任何文件
        let report = audit_directory(&dir, &cfg, (), &token).unwrap();
        assert_eq!(report.total_files, 8);
        let skipped = report.files.iter().filter(|f| f.status == FileStatus::Skipped).count();
        assert_eq!(skipped, 8, "取消后所有文件应标记为未检查");
        assert!(report.files.iter().all(|f| f.violations.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancellation_flag_is_idempotent_and_shared() {
        let token = Arc::new(CancellationToken::new());
        assert!(!token.is_cancelled());
        token.cancel();
        token.cancel();
        assert!(token.is_cancelled());
        let t2 = token.clone();
        std::thread::spawn(move || t2.is_cancelled()).join().unwrap();
    }

    #[test]
    fn no_docx_in_dir_returns_zero_total_but_valid_report() {
        let dir = unique_tmp_dir("empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("说明.txt"), "不是论文".as_bytes()).unwrap();
        let rec = Arc::new(Recorder::default());
        struct Proxy(Arc<Recorder>);
        impl AuditProgress for Proxy {
            fn on_file_started(&self, f: &Path) { self.0.on_file_started(f) }
            fn on_file_finished(&self, f: &Path, n: usize) { self.0.on_file_finished(f, n) }
            fn on_log(&self, m: String) { self.0.on_log(m) }
        }
        let report = audit_directory(&dir, &shipped_cfg(), Proxy(rec.clone()), &CancellationToken::new()).unwrap();
        assert_eq!(report.total_files, 0);
        assert!(rec.logs.lock().unwrap().iter().any(|l| l.contains("没有找到")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_directory_returns_friendly_invalid_path() {
        let e = audit_directory(Path::new("/definitely/not/a/dir"), &shipped_cfg(), (), &CancellationToken::new()).unwrap_err();
        assert!(matches!(e, CoreError::InvalidPath { .. }));
        assert!(e.user_message().contains("文件夹"), "{}", e.user_message());
    }

    #[test]
    fn timestamp_format_sane() {
        let ts = now_display();
        // YYYY-MM-DD HH:MM:SS UTC
        assert!(ts.len() >= 20 && ts.as_bytes()[4] == b'-' && ts.contains("UTC"), "{ts}");
    }
}
