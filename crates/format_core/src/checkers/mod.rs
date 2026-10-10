//! checkers —— 各 RuleKind 对应的检查器实现与注册表。
//!
//! 组织方式：
//! - [`page`]      页面级：page_size / page_margins / page_numbering
//! - [`font`]      run 级：run_font_size / run_font_family / font_allow_list
//! - [`paragraph`] 段落级：line_spacing / indent / alignment / bold / text_length / keyword_list
//! - [`structure`] 文档结构：sequence / heading_numbering / citation_order
//! - [`caption`]   题注：caption（图/表）
//! - [`reference`] 参考文献：reference_list
//! - [`typography`]标点与空格：punctuation / cjk_latin_spacing
//!
//! 约定：所有检查器无内部可变状态（CompiledRegex 在 build 时编译一次），
//! 因此天然满足 `Send + Sync`，可在 rayon 中并发执行。

pub mod caption;
pub mod font;
pub mod page;
pub mod paragraph;
pub mod reference;
pub mod structure;
pub mod typography;

use crate::dsl::{
    Location, Paragraph, Rule, RuleConfig, RuleKind, Severity, TargetSpec, Violation,
};
use crate::error::{CoreError, CoreResult};
use regex::Regex;

/// 一条规则构造出的检查器。
pub struct CompiledChecker {
    pub rule: Rule,
    checker: Box<dyn crate::dsl::Checker>,
    /// target.text_regex 在构建期编译一次，热路径零重编译（审查项：DoS/性能）。
    text_regex: Option<Regex>,
}

impl CompiledChecker {
    /// 测试专用：注入自定义 Checker（验证引擎容错/调度逻辑）。
    #[cfg(test)]
    pub(crate) fn for_test(rule: Rule, checker: Box<dyn crate::dsl::Checker>) -> Self {
        let text_regex = rule
            .target
            .as_ref()
            .and_then(|t| t.text_regex.as_deref())
            .and_then(|pat| Regex::new(pat).ok());
        Self {
            rule,
            checker,
            text_regex,
        }
    }

    /// 构建期编译 target 正则；非法正则在加载期已校验，此处失败按“不命中”处理并保留原文兜底。
    pub(crate) fn new(rule: Rule, checker: Box<dyn crate::dsl::Checker>) -> Self {
        let text_regex = rule
            .target
            .as_ref()
            .and_then(|t| t.text_regex.as_deref())
            .and_then(|pat| Regex::new(pat).ok());
        Self {
            rule,
            checker,
            text_regex,
        }
    }

    pub fn compiled_text_regex(&self) -> Option<&Regex> {
        self.text_regex.as_ref()
    }

    pub fn rule_id(&self) -> &str {
        self.rule.id.as_str()
    }
    pub fn is_document_level(&self) -> bool {
        self.rule.kind.is_document_level()
    }
    pub fn target(&self) -> Option<&TargetSpec> {
        self.rule.target.as_ref()
    }
    pub fn check_document(&self, ctx: &crate::dsl::AuditContext<'_>) -> Vec<Violation> {
        self.checker.check_document(ctx)
    }
    pub fn check_paragraph(
        &self,
        ctx: &crate::dsl::AuditContext<'_>,
        p: &Paragraph,
    ) -> Vec<Violation> {
        self.checker.check_paragraph(ctx, p)
    }
}

/// 编译单条规则为检查器；正则非法时返回中文可解释错误。
pub fn build_checker(rule: &Rule, config: &RuleConfig) -> CoreResult<CompiledChecker> {
    let checker: Box<dyn crate::dsl::Checker> = match &rule.kind {
        RuleKind::PageSize { expected } => Box::new(page::PageSizeChecker {
            expected: expected.clone(),
        }),
        RuleKind::PageMargins { expected_cm } => Box::new(page::PageMarginsChecker {
            expected: expected_cm.clone(),
        }),
        RuleKind::PageNumbering { expected } => Box::new(page::PageNumberingChecker {
            expected: expected.clone(),
        }),
        RuleKind::RunFontSize {
            expected_pt,
            allowed_pt,
            range_pt,
            apply_to,
        } => Box::new(font::RunFontSizeChecker {
            expected_pt: *expected_pt,
            allowed_pt: allowed_pt.clone(),
            range_pt: range_pt.clone(),
            apply_to: apply_to.clone(),
        }),
        RuleKind::RunFontFamily {
            text_class,
            expected_fonts,
            blocked_fonts,
        } => Box::new(font::RunFontFamilyChecker {
            text_class: text_class.clone(),
            expected_fonts: expected_fonts.clone(),
            blocked_fonts: blocked_fonts.clone(),
        }),
        RuleKind::FontAllowList {
            mode,
            allowed_fonts,
            blocked_fonts,
        } => Box::new(font::FontAllowListChecker {
            mode: mode.clone(),
            allowed_fonts: allowed_fonts.clone(),
            blocked_fonts: blocked_fonts.clone(),
            policy_blocklist: config.font_policy.blocked_decorative_fonts.clone(),
            policy_allowlist: config.font_policy.allowed_fonts.clone(),
        }),
        RuleKind::LineSpacing { allowed } => Box::new(paragraph::LineSpacingChecker {
            allowed: allowed.clone(),
        }),
        RuleKind::Indent {
            first_line_chars,
            hanging_chars,
            left_chars,
        } => Box::new(paragraph::IndentChecker {
            first_line_chars: *first_line_chars,
            hanging_chars: *hanging_chars,
            left_chars: *left_chars,
        }),
        RuleKind::Alignment { expected } => Box::new(paragraph::AlignmentChecker {
            expected: *expected,
        }),
        RuleKind::Bold { expected } => Box::new(paragraph::BoldChecker {
            expected: *expected,
        }),
        RuleKind::TextLength {
            min,
            max,
            count_mode,
        } => Box::new(paragraph::TextLengthChecker {
            min: *min,
            max: *max,
            count_mode: count_mode.clone(),
        }),
        RuleKind::KeywordList {
            min_count,
            max_count,
            separators,
            allow_trailing_separator,
        } => Box::new(paragraph::KeywordListChecker {
            min_count: *min_count,
            max_count: *max_count,
            separators: separators.clone(),
            allow_trailing: *allow_trailing_separator,
        }),
        RuleKind::Punctuation { mode, exceptions } => Box::new(typography::PunctuationChecker {
            mode: mode.clone(),
            exception_regexes: compile_exception_patterns(&rule.id, exceptions)?,
        }),
        RuleKind::CjkLatinSpacing { mode } => {
            Box::new(typography::CjkLatinSpacingChecker { mode: mode.clone() })
        }
        RuleKind::HeadingNumbering {
            allowed_schemes,
            require_consistent,
            check_level_match,
        } => {
            let (cn_res, arabic_res) = structure::heading_default_regexes();
            Box::new(structure::HeadingNumberingChecker {
                allowed_schemes: allowed_schemes.clone(),
                require_consistent: *require_consistent,
                check_level_match: *check_level_match,
                cn_res,
                arabic_res,
            })
        }
        RuleKind::Sequence { anchors, strict } => Box::new(structure::SequenceChecker {
            anchors: anchors.clone(),
            strict: *strict,
            anchor_regexes: compile_anchor_regexes(&rule.id, anchors)?,
        }),
        RuleKind::CitationOrder {
            require_ascending,
            allow_repeated,
        } => Box::new(structure::CitationOrderChecker {
            require_ascending: *require_ascending,
            allow_repeated: *allow_repeated,
        }),
        RuleKind::Caption {
            caption_kind,
            pattern,
            expected_alignment,
            expected_position,
            numbering,
        } => Box::new(caption::CaptionChecker {
            kind: caption_kind.clone(),
            pattern: compile_regex(&rule.id, pattern)?,
            expected_alignment: *expected_alignment,
            expected_position: expected_position.clone(),
            numbering: numbering.clone(),
        }),
        RuleKind::ReferenceList {
            entry_pattern,
            required_type_tags,
            check_continuous,
            check_cited,
        } => Box::new(reference::ReferenceListChecker {
            entry_pattern: compile_regex(&rule.id, entry_pattern)?,
            required_type_tags: required_type_tags.clone(),
            check_continuous: *check_continuous,
            check_cited: *check_cited,
        }),
    };
    Ok(CompiledChecker::new(rule.clone(), checker))
}

/// 根据配置构建全部启用的检查器（跳过 enabled=false）。
pub fn build_checkers(config: &RuleConfig) -> CoreResult<Vec<CompiledChecker>> {
    let mut out = Vec::with_capacity(config.rules.len());
    for rule in &config.rules {
        if !rule.enabled {
            continue;
        }
        out.push(build_checker(rule, config)?);
    }
    Ok(out)
}

fn compile_regex(rule_id: &str, pattern: &str) -> CoreResult<Regex> {
    Regex::new(pattern).map_err(|e| CoreError::InvalidRegex {
        rule_id: rule_id.to_string(),
        source: e,
    })
}

fn compile_exception_patterns(rule_id: &str, exceptions: &[String]) -> CoreResult<Vec<Regex>> {
    exceptions
        .iter()
        .map(|p| compile_regex(rule_id, p))
        .collect()
}

fn compile_anchor_regexes(
    rule_id: &str,
    anchors: &[crate::dsl::SequenceAnchor],
) -> CoreResult<Vec<Option<Regex>>> {
    anchors
        .iter()
        .map(|a| match &a.regex {
            Some(p) => compile_regex(rule_id, p).map(Some),
            None => Ok(None),
        })
        .collect()
}

// ---------- 公共小工具（各 checker 复用） ----------

/// 生成一条违规记录（severity/manual_review/suggestion 取自规则本身）。
pub(crate) fn violation_for(
    rule: &Rule,
    doc_path: &std::path::Path,
    location: Location,
    message: String,
    expected: Option<String>,
    actual: Option<String>,
    excerpt: Option<String>,
    confidence: f32,
) -> Violation {
    Violation {
        rule_id: rule.id.clone(),
        severity: rule.severity,
        file_path: doc_path.to_path_buf(),
        location,
        message,
        expected,
        actual,
        excerpt,
        suggestion: rule.suggestion.clone(),
        confidence,
        requires_manual_review: rule.manual_review,
    }
}

/// 降级提示（未能识别 → info + 建议人工检查）。
pub(crate) fn manual_hint(
    rule: &Rule,
    doc_path: &std::path::Path,
    location: Location,
    message: String,
    excerpt: Option<String>,
) -> Violation {
    let mut v = violation_for(
        rule,
        doc_path,
        location,
        message,
        None,
        Some("未能识别".into()),
        excerpt,
        0.3,
    );
    v.severity = Severity::Info;
    v.requires_manual_review = true;
    v
}

/// 从段落提取违规位置信息（全局段落号 + 页码估算）。
pub(crate) fn loc_of(p: &Paragraph) -> Location {
    Location {
        paragraph_index: Some(p.global_index),
        run_index: None,
        page_hint: p.page_hint,
        ..Default::default()
    }
}

/// 截断文本片段用于 excerpt（默认 40 字符）。
pub(crate) fn excerpt_of(text: &str, max_chars: usize) -> String {
    let t: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{t}…")
    } else {
        t
    }
}

/// 数字展示：整数不带小数点，否则保留一位。
pub(crate) fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.1}", v)
    }
}

/// 字号期望文案：“小四（12 pt）”或“12 pt”。
pub(crate) fn fmt_size_expected(config: &RuleConfig, pt: f64) -> String {
    match config.font_size_alias_cn(pt) {
        Some(alias) => format!("{alias}（{} pt）", fmt_num(pt)),
        None => format!("{} pt", fmt_num(pt)),
    }
}

/// 实际值文案：“约 10.5 pt”。
pub(crate) fn fmt_actual_pt(pt: f64) -> String {
    format!("约 {} pt", fmt_num(pt))
}

pub(crate) use crate::dsl::within;
