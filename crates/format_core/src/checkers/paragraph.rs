//! paragraph —— 段落级规则：line_spacing / indent / alignment / bold / text_length / keyword_list。

use crate::checkers::{excerpt_of, fmt_num, violation_for, within};
use crate::dsl::{
    AuditContext, Checker, CountMode, Indent, LineSpacing, LineSpacingExpectation, Location,
    Paragraph, ParagraphRole, Severity, Violation,
};

// ---------------- line_spacing ----------------

pub struct LineSpacingChecker {
    pub allowed: Vec<LineSpacingExpectation>,
}

impl LineSpacingChecker {
    fn accepts(&self, ls: &LineSpacing, tol_mult: f64, tol_pt: f64) -> bool {
        self.allowed.iter().any(|exp| match (ls, exp.rule.as_str()) {
            (LineSpacing::Multiple { factor }, "multiple") => {
                exp.factor.map(|f| within(*factor, f, tol_mult)).unwrap_or(false)
            }
            (LineSpacing::Exact { points }, "exact") => {
                let lo = exp.min_points.unwrap_or(*points);
                let hi = exp.max_points.unwrap_or(*points);
                *points >= lo - tol_pt && *points <= hi + tol_pt
            }
            (LineSpacing::AtLeast { points }, "atLeast") | (LineSpacing::AtLeast { points }, "at_least") => {
                let lo = exp.min_points.unwrap_or(*points);
                let hi = exp.max_points.unwrap_or(f64::INFINITY);
                *points >= lo - tol_pt && *points <= hi + tol_pt
            }
            _ => false,
        })
    }

    fn describe(&self) -> String {
        self.allowed
            .iter()
            .map(|e| match e.rule.as_str() {
                "multiple" => format!("{} 倍行距", fmt_num(e.factor.unwrap_or(0.0))),
                "exact" => format!(
                    "固定值 {}–{} pt",
                    fmt_num(e.min_points.unwrap_or(0.0)),
                    fmt_num(e.max_points.unwrap_or(0.0))
                ),
                _ => format!("最小 {} pt", fmt_num(e.min_points.unwrap_or(0.0))),
            })
            .collect::<Vec<_>>()
            .join(" 或 ")
    }
}

impl Checker for LineSpacingChecker {
    fn rule_id(&self) -> &str {
        "line_spacing"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        if p.text.trim().is_empty() {
            return Vec::new();
        }
        let eff = p.effective.clone().unwrap_or_default();
        if let Some(ls) = &eff.line_spacing {
            return self.judge(ctx, p, ls);
        }
        if let Some(direct) = &p.direct.line_spacing {
            return self.judge(ctx, p, direct);
        }
        vec![crate::checkers::manual_hint(
            ctx.rule,
            &ctx.document.path,
            loc_of(p),
            "此处未能识别行距，建议人工检查。".into(),
            Some(excerpt_of(p.text.trim(), 30)),
        )]
    }
}

impl LineSpacingChecker {
    fn judge(&self, ctx: &AuditContext<'_>, p: &Paragraph, ls: &LineSpacing) -> Vec<Violation> {
        let t = ctx.config.tolerance();
        if self.accepts(ls, t.line_spacing_multiple, t.line_spacing_pt) {
            return Vec::new();
        }
        let actual = match ls {
            LineSpacing::Multiple { factor } => format!("约 {} 倍行距", fmt_num(*factor)),
            LineSpacing::Exact { points } => format!("固定值 {} pt", fmt_num(*points)),
            LineSpacing::AtLeast { points } => format!("最小 {} pt", fmt_num(*points)),
        };
        vec![violation_for(
            ctx.rule,
            &ctx.document.path,
            loc_of(p),
            format!("{}当前为{actual}。", ctx.rule.message.trim_end_matches('。')),
            Some(self.describe()),
            Some(actual),
            Some(excerpt_of(p.text.trim(), 30)),
            0.9,
        )]
    }
}

// ---------------- indent ----------------

pub struct IndentChecker {
    pub first_line_chars: Option<f64>,
    pub hanging_chars: Option<f64>,
    pub left_chars: Option<f64>,
}

/// 段落估算字号：优先第一个非空 run 的有效字号，其次文档默认。
fn paragraph_font_size(ctx: &AuditContext<'_>, p: &Paragraph) -> Option<f64> {
    p.runs
        .iter()
        .find(|r| !r.text.trim().is_empty())
        .and_then(|r| r.effective.as_ref().and_then(|e| e.font_size_pt).or(r.direct.font_size_pt))
        .or_else(|| ctx.document.doc_defaults.as_ref().and_then(|d| d.run_props.font_size_pt))
}

#[derive(Clone, Copy)]
enum IndentWhich {
    FirstLine,
    Hanging,
    Left,
}

/// 取实际缩进字符数；绝对单位按字号估算；无法估算返回 None。
fn actual_indent_chars(ctx: &AuditContext<'_>, p: &Paragraph, which: IndentWhich) -> Option<f64> {
    let eff = p.effective.clone().unwrap_or_default();
    let ind = match which {
        IndentWhich::FirstLine => eff.first_line_indent.clone().or(p.direct.first_line_indent.clone()),
        IndentWhich::Hanging => eff.hanging_indent.clone().or(p.direct.hanging_indent.clone()),
        IndentWhich::Left => eff.left_indent.clone().or(p.direct.left_indent.clone()),
    }?;
    if let Indent::Characters { value } = &ind {
        return Some(*value);
    }
    let size = paragraph_font_size(ctx, p)?;
    ind.approx_chars(size)
}

impl Checker for IndentChecker {
    fn rule_id(&self) -> &str {
        "indent"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let mut out = Vec::new();
        let text = p.text.trim();
        if text.is_empty() {
            return out;
        }
        let tol = ctx.config.tolerance().indent_chars;

        // 首行缩进：跳过标题、题注、单行短段落（不足一行不强制缩进）。
        if let Some(want) = self.first_line_chars {
            let skip = matches!(
                p.role,
                ParagraphRole::Heading { .. }
                    | ParagraphRole::FigureCaption
                    | ParagraphRole::TableCaption
                    | ParagraphRole::Title
            );
            let one_line_short = text.chars().count() < 28; // 保守阈值：明显不足一行
            if !skip && !one_line_short {
                match actual_indent_chars(ctx, p, IndentWhich::FirstLine) {
                    None => out.push(crate::checkers::manual_hint(
                        ctx.rule,
                        &ctx.document.path,
                        loc_of(p),
                        "此处缩进无法自动判断（缺少字号信息），建议人工检查。".into(),
                        Some(excerpt_of(text, 30)),
                    )),
                    Some(got) if (got - want).abs() > tol => out.push(violation_for(
                        ctx.rule,
                        &ctx.document.path,
                        loc_of(p),
                        format!("{}当前约为 {} 字符。", ctx.rule.message.trim_end_matches('。'), fmt_num(got)),
                        Some(format!("{want} 字符")),
                        Some(format!("约 {} 字符", fmt_num(got))),
                        Some(excerpt_of(text, 30)),
                        0.85,
                    )),
                    Some(_) => {}
                }
            }
        }

        // 悬挂缩进（参考文献常用）
        if let Some(want) = self.hanging_chars {
            match actual_indent_chars(ctx, p, IndentWhich::Hanging) {
                None => out.push(crate::checkers::manual_hint(
                    ctx.rule,
                    &ctx.document.path,
                    loc_of(p),
                    "此处悬挂缩进无法自动判断，建议人工检查。".into(),
                    Some(excerpt_of(text, 30)),
                )),
                Some(got) if (got - want).abs() > tol => out.push(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    loc_of(p),
                    format!("{}当前约为 {} 字符。", ctx.rule.message.trim_end_matches('。'), fmt_num(got)),
                    Some(format!("{want} 字符")),
                    Some(format!("约 {} 字符", fmt_num(got))),
                    Some(excerpt_of(text, 30)),
                    0.85,
                )),
                Some(_) => {}
            }
        }

        // 左缩进
        if let Some(want) = self.left_chars {
            if let Some(got) = actual_indent_chars(ctx, p, IndentWhich::Left) {
                if (got - want).abs() > tol {
                    out.push(violation_for(
                        ctx.rule,
                        &ctx.document.path,
                        loc_of(p),
                        format!("{}当前约为 {} 字符。", ctx.rule.message.trim_end_matches('。'), fmt_num(got)),
                        Some(format!("{want} 字符")),
                        Some(format!("约 {} 字符", fmt_num(got))),
                        Some(excerpt_of(text, 30)),
                        0.85,
                    ));
                }
            }
        }
        out
    }
}

// ---------------- alignment ----------------

pub struct AlignmentChecker {
    pub expected: crate::dsl::Alignment,
}

impl Checker for AlignmentChecker {
    fn rule_id(&self) -> &str {
        "alignment"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        if p.text.trim().is_empty() {
            return Vec::new();
        }
        let eff = p.effective.clone().unwrap_or_default();
        let got = eff.alignment.or(p.direct.alignment);
        match got {
            None | Some(crate::dsl::Alignment::Unknown) => Vec::new(), // 未识别不报 error
            Some(a) if a == self.expected => Vec::new(),
            Some(a) => vec![violation_for(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                format!("{}当前为{}。", ctx.rule.message.trim_end_matches('。'), a.label_cn()),
                Some(format!("应{}", self.expected.label_cn())),
                Some(a.label_cn().to_string()),
                Some(excerpt_of(p.text.trim(), 30)),
                0.9,
            )],
        }
    }
}

// ---------------- bold ----------------

pub struct BoldChecker {
    pub expected: bool,
}

impl Checker for BoldChecker {
    fn rule_id(&self) -> &str {
        "bold"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        if p.text.trim().is_empty() || p.runs.is_empty() {
            return Vec::new();
        }
        let all_bold = p.runs.iter().all(|r| {
            if r.text.trim().is_empty() {
                return true;
            }
            r.effective.as_ref().map(|e| e.bold).unwrap_or(r.direct.bold.unwrap_or(false))
        });
        if all_bold == self.expected {
            Vec::new()
        } else {
            vec![violation_for(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                ctx.rule.message.clone(),
                Some(if self.expected { "加粗".into() } else { "不加粗".into() }),
                Some(if all_bold { "加粗".into() } else { "未加粗".into() }),
                Some(excerpt_of(p.text.trim(), 30)),
                0.85,
            )]
        }
    }
}

// ---------------- text_length ----------------

pub struct TextLengthChecker {
    pub min: usize,
    pub max: usize,
    pub count_mode: CountMode,
}

/// 按模式统计“字数”（第一版口径：标点不计入，文案中已说明）。
pub(crate) fn count_text(text: &str, mode: &CountMode) -> usize {
    match mode {
        CountMode::VisibleChars => text.chars().filter(|c| !c.is_whitespace()).count(),
        CountMode::CjkChars => text.chars().filter(|c| crate::dsl::is_cjk_ideograph(*c)).count(),
        CountMode::CjkCharsPlusLatinWords => {
            let mut n = 0usize;
            let mut in_word = false;
            for c in text.chars() {
                if crate::dsl::is_cjk_ideograph(c) {
                    n += 1;
                    in_word = false;
                } else if c.is_ascii_alphanumeric() {
                    if !in_word {
                        n += 1; // 连续英文单词/数字串各计 1
                        in_word = true;
                    }
                } else {
                    in_word = false;
                }
            }
            n
        }
    }
}

/// 去除 “关键词：/摘要：” 等标签前缀（含全角空格与冒号变体）。
pub(crate) fn strip_label_prefix(text: &str) -> String {
    let re = regex::Regex::new(r"^[\s\u{3000}]*(摘\s*要|关\s*键\s*词|Keywords?|Abstract)[\s\u{3000}]*[:：][\s\u{3000}]*").unwrap();
    re.replace(text, "").into_owned()
}

impl Checker for TextLengthChecker {
    fn rule_id(&self) -> &str {
        "text_length"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let text = p.text.trim();
        if text.is_empty() {
            return vec![crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                "未识别到相应正文内容，建议人工检查。".into(),
                None,
            )];
        }
        let body = strip_label_prefix(text);
        let n = count_text(&body, &self.count_mode);
        if n < self.min {
            return vec![length_violation(
                ctx,
                p,
                &format!("内容偏短：建议不少于 {} 个词字（不含标点），当前约 {n} 个。", self.min),
                n,
            )];
        }
        if n > self.max {
            return vec![length_violation(
                ctx,
                p,
                &format!("内容偏长：建议不超过 {} 个词字（不含标点），当前约 {n} 个。", self.max),
                n,
            )];
        }
        Vec::new()
    }
}

fn length_violation(ctx: &AuditContext<'_>, p: &Paragraph, msg: &str, n: usize) -> Violation {
    violation_for(
        ctx.rule,
        &ctx.document.path,
        loc_of(p),
        msg.to_string(),
        Some(format!("{}–{}", ctx.rule.kind_min(), ctx.rule.kind_max())),
        Some(format!("约 {n}")),
        Some(excerpt_of(p.text.trim(), 30)),
        0.85,
    )
}

// ---------------- keyword_list ----------------

pub struct KeywordListChecker {
    pub min_count: usize,
    pub max_count: usize,
    pub separators: Vec<String>,
    pub allow_trailing: bool,
}

impl Checker for KeywordListChecker {
    fn rule_id(&self) -> &str {
        "keyword_list"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let text = strip_label_prefix(p.text.trim());
        if text.is_empty() {
            return vec![crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                "未识别到关键词内容，建议人工检查。".into(),
                None,
            )];
        }
        let seps: Vec<char> = self.separators.iter().filter_map(|s| s.chars().next()).collect();
        let trailing = text.chars().next_back().map(|c| seps.contains(&c)).unwrap_or(false);
        let cleaned = if trailing { text.trim_end_matches(seps.as_slice()).to_string() } else { text.clone() };
        let parts: Vec<&str> = cleaned.split(seps.as_slice()).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        let n = parts.len();

        let mut out = Vec::new();
        if n < self.min_count || n > self.max_count {
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                format!("关键词数量建议在 {}–{} 个，当前识别到 {n} 个。", self.min_count, self.max_count),
                Some(format!("{}–{} 个", self.min_count, self.max_count)),
                Some(format!("{n} 个")),
                Some(excerpt_of(p.text.trim(), 40)),
                0.85,
            ));
        }
        if trailing && !self.allow_trailing {
            let mut v = violation_for(
                ctx.rule,
                &ctx.document.path,
                loc_of(p),
                "关键词末尾多了一个分隔符（如分号），建议删除。".to_string(),
                Some("末尾无分隔符".into()),
                Some("末尾有分隔符".into()),
                Some(excerpt_of(p.text.trim(), 40)),
                0.8,
            );
            v.severity = Severity::Info;
            out.push(v);
        }
        out
    }
}

pub(crate) fn loc_of(p: &Paragraph) -> Location {
    Location {
        section_index: Some(p.section_index),
        block_index: Some(p.block_index),
        paragraph_index: Some(p.global_index),
        run_index: None,
        page_hint: p.page_hint,
        object_ref: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::font::tests::{empty_doc, para, run_with};
    use crate::dsl::{DirectParagraphProps, EffectiveParagraphFormat};
    use crate::rules::tests_helper::config_from_json;

    fn para_ls(text: &str, role: ParagraphRole, ls: Option<LineSpacing>) -> Paragraph {
        let mut p = para(vec![run_with(text, 0, Some(12.0), None, None)], role);
        p.effective = Some(EffectiveParagraphFormat { line_spacing: ls, ..Default::default() });
        p
    }

    #[test]
    fn line_spacing_multiple_and_exact_pass_wide_fails() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-line-spacing").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        assert!(c
            .check_paragraph(&ctx, &para_ls("正文内容示例。", ParagraphRole::Body, Some(LineSpacing::Multiple { factor: 1.5 })))
            .is_empty());
        assert!(c
            .check_paragraph(&ctx, &para_ls("正文内容示例。", ParagraphRole::Body, Some(LineSpacing::Exact { points: 20.0 })))
            .is_empty());
        let vs = c.check_paragraph(&ctx, &para_ls("正文内容示例。", ParagraphRole::Body, Some(LineSpacing::Exact { points: 28.0 })));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].message.contains("行距"));
    }

    #[test]
    fn line_spacing_unknown_is_info() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-line-spacing").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let vs = c.check_paragraph(&ctx, &para_ls("正文内容示例。", ParagraphRole::Body, None));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
    }

    fn para_indent(text: &str, role: ParagraphRole, first: Option<Indent>) -> Paragraph {
        let mut p = para(vec![run_with(text, 0, Some(12.0), None, None)], role);
        p.effective = Some(EffectiveParagraphFormat { first_line_indent: first, ..Default::default() });
        p
    }

    #[test]
    fn first_line_indent_2_chars_passes_0_warns() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-first-line-indent").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let long = "教育公平是教育发展质量的重要体现，需要在制度层面持续推进均衡化安排。";
        assert!(c
            .check_paragraph(&ctx, &para_indent(long, ParagraphRole::Body, Some(Indent::Characters { value: 2.0 })))
            .is_empty());
        let vs = c.check_paragraph(&ctx, &para_indent(long, ParagraphRole::Body, Some(Indent::Characters { value: 0.0 })));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        // twip 估算：480 twip = 24pt = 2 字符（12pt 字号）
        assert!(c.check_paragraph(&ctx, &para_indent(long, ParagraphRole::Body, Some(Indent::Twips { value: 480.0 }))).is_empty());
        // 短段落不误报
        assert!(c
            .check_paragraph(&ctx, &para_indent("很短的句子。", ParagraphRole::Body, Some(Indent::Characters { value: 0.0 })))
            .is_empty());
    }

    #[test]
    fn hanging_indent_for_references() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "reference-hanging-indent").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let mut p = para(
            vec![run_with("[1] 张三. 教育公平研究[M]. 北京: 教育出版社, 2020.", 0, Some(12.0), None, None)],
            ParagraphRole::ReferenceEntry,
        );
        p.effective = Some(EffectiveParagraphFormat { hanging_indent: Some(Indent::Characters { value: 2.0 }), ..Default::default() });
        assert!(c.check_paragraph(&ctx, &p).is_empty());
        p.effective = Some(EffectiveParagraphFormat { hanging_indent: Some(Indent::Characters { value: 0.0 }), ..Default::default() });
        let vs = c.check_paragraph(&ctx, &p);
        assert_eq!(vs.len(), 1);
    }

    #[test]
    fn alignment_heading_center() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "heading1-align").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let mut p = para(vec![run_with("一、绪论", 0, Some(16.0), None, None)], ParagraphRole::Heading { level: 1 });
        p.effective = Some(EffectiveParagraphFormat { alignment: Some(crate::dsl::Alignment::Center), ..Default::default() });
        assert!(c.check_paragraph(&ctx, &p).is_empty());
        p.effective = Some(EffectiveParagraphFormat { alignment: Some(crate::dsl::Alignment::Left), ..Default::default() });
        let vs = c.check_paragraph(&ctx, &p);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        // alignment 为 None 时不报错
        p.effective = None;
        p.direct = DirectParagraphProps::default();
        assert!(c.check_paragraph(&ctx, &p).is_empty());
    }

    #[test]
    fn abstract_length_counts_cjk_plus_latin_words() {
        assert_eq!(count_text("教育人工智能发展研究", &CountMode::CjkCharsPlusLatinWords), 10);
        assert_eq!(count_text("AI 与 education 公平 2026", &CountMode::CjkCharsPlusLatinWords), 5); // AI+education+与+公平+2026
        assert_eq!(strip_label_prefix("摘要：本文研究了……"), "本文研究了……");
    }

    #[test]
    fn abstract_length_250_passes_120_warns() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "abstract-length").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let text250 = "研".repeat(250);
        assert!(c.check_paragraph(&ctx, &para(&format!("摘要：{text250}"), ParagraphRole::AbstractBody)).is_empty());
        let vs = c.check_paragraph(&ctx, &para(format!("摘要：{}", "研".repeat(120)), ParagraphRole::AbstractBody));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].message.contains("不少于 200"));
    }

    #[test]
    fn keyword_count_3_passes_2_and_6_warn() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "keyword-count").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        assert!(c.check_paragraph(&ctx, &para("关键词：教育公平；政策分析；实证研究", ParagraphRole::KeywordsBody)).is_empty());
        let vs = c.check_paragraph(&ctx, &para("关键词：教育公平；政策分析", ParagraphRole::KeywordsBody));
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("当前识别到 2 个"));
        let vs = c.check_paragraph(&ctx, &para("关键词：A；B；C；D；E；F", ParagraphRole::KeywordsBody));
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("当前识别到 6 个"));
    }

    #[test]
    fn keyword_trailing_separator_is_info_not_error() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "keyword-separator-style").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let vs = c.check_paragraph(&ctx, &para("关键词：教育公平；政策分析；实证研究；", ParagraphRole::KeywordsBody));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
        assert!(vs[0].message.contains("末尾"));
    }
}
