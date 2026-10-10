//! font —— run 级规则：run_font_size / run_font_family / font_allow_list。
//!
//! 聚合策略：一个段落内多个违规 run 只产出一条 Violation（excerpt 取第一个违规 run），
//! 由 audit 层再做 (paragraph, rule) 去重，避免刷屏。

use crate::checkers::{excerpt_of, fmt_actual_pt, fmt_size_expected, violation_for, within};
use crate::dsl::{
    classify_text, AuditContext, Checker, EffectiveRunFormat, Location, Paragraph, RunApply,
    RuleConfig, Severity, TextClassFilter, Violation,
};

/// 取 run 的有效格式；求解器未给出时用直接格式兜底。
pub(crate) fn effective_of(run: &crate::dsl::Run) -> EffectiveRunFormat {
    run.effective.clone().unwrap_or_else(|| EffectiveRunFormat {
        font_size_pt: run.direct.font_size_pt,
        ascii_font: run.direct.ascii_font.clone(),
        east_asian_font: run.direct.east_asian_font.clone(),
        bold: run.direct.bold.unwrap_or(false),
        italic: run.direct.italic.unwrap_or(false),
        underline: run.direct.underline.unwrap_or(false),
        highlight: run.direct.highlight.clone(),
        color: run.direct.color.clone(),
        sources: Default::default(),
    })
}

// ---------------- run_font_size ----------------

pub struct RunFontSizeChecker {
    pub expected_pt: Option<f64>,
    pub allowed_pt: Option<Vec<f64>>,
    pub range_pt: Option<crate::dsl::RangeF64>,
    pub apply_to: RunApply,
}

impl RunFontSizeChecker {
    fn describe(&self, config: &RuleConfig) -> String {
        if let Some(pt) = self.expected_pt {
            return fmt_size_expected(config, pt);
        }
        if let Some(list) = &self.allowed_pt {
            let parts: Vec<String> = list.iter().map(|p| fmt_size_expected(config, *p)).collect();
            return parts.join(" 或 ");
        }
        if let Some(r) = &self.range_pt {
            return format!("{}–{} pt", crate::checkers::fmt_num(r.min), crate::checkers::fmt_num(r.max));
        }
        "未能确定期望字号".into()
    }

    fn accepts(&self, size: f64, tol: f64) -> bool {
        if let Some(pt) = self.expected_pt {
            return within(size, pt, tol);
        }
        if let Some(list) = &self.allowed_pt {
            return list.iter().any(|p| within(size, *p, tol));
        }
        if let Some(r) = &self.range_pt {
            return size >= r.min - tol && size <= r.max + tol;
        }
        true
    }
}

impl Checker for RunFontSizeChecker {
    fn rule_id(&self) -> &str {
        "run_font_size"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let mut out = Vec::new();
        let tol = ctx.config.tolerance().font_size_pt;
        let runs: Vec<&crate::dsl::Run> = match self.apply_to {
            RunApply::AllRuns => p.runs.iter().collect(),
            RunApply::NonEmptyRuns => p.runs.iter().filter(|r| !r.text.trim().is_empty()).collect(),
            RunApply::PrimaryRun => p.runs.iter().filter(|r| !r.text.trim().is_empty()).take(1).collect(),
        };
        if runs.is_empty() {
            return out;
        }
        let mut first_bad: Option<(usize, f64)> = None;
        let mut any_missing = false;
        for r in &runs {
            match effective_of(r).font_size_pt {
                None => any_missing = true,
                Some(sz) if !self.accepts(sz, tol) => {
                    if first_bad.is_none() {
                        first_bad = Some((r.index, sz));
                    }
                }
                Some(_) => {}
            }
        }
        if let Some((idx, sz)) = first_bad {
            let excerpt = runs
                .iter()
                .find(|r| r.index == idx)
                .map(|r| excerpt_of(r.text.trim(), 30))
                .filter(|s| !s.is_empty());
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(p.global_index),
                    block_index: Some(p.block_index),
                    section_index: Some(p.section_index),
                    run_index: Some(idx),
                    page_hint: p.page_hint,
                    object_ref: None,
                },
                format!("{}当前为{}。", ctx.rule.message.trim_end_matches('。'), fmt_actual_pt(sz)),
                Some(self.describe(ctx.config)),
                Some(fmt_actual_pt(sz)),
                excerpt,
                0.9,
            ));
        } else if any_missing {
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(p.global_index),
                    block_index: Some(p.block_index),
                    section_index: Some(p.section_index),
                    ..Default::default()
                },
                "此处未能识别字号，建议人工检查。".into(),
                Some(excerpt_of(p.text.trim(), 30)).filter(|s| !s.is_empty()),
            ));
        }
        out
    }
}

// ---------------- run_font_family ----------------

/// 字体名归一化：忽略大小写/空格/前导 @（东亚字体常见）。
fn norm_font(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "").trim_start_matches('@').to_string()
}

/// 别名表：宋体/SimSun、黑体/SimHei、楷体/KaiTi 等视为同一字体族。
fn font_alias_group(norm: &str) -> &'static [&'static str] {
    const NONE: &[&str] = &[];
    match norm {
        "宋体" | "simsun" | "中易宋体" => &["宋体", "simsun", "中易宋体"],
        "黑体" | "simhei" | "中易黑体" => &["黑体", "simhei", "中易黑体"],
        "楷体" | "kaiti" | "楷体_gb2312" | "kaiti_gb2312" => &["楷体", "kaiti", "楷体_gb2312", "kaiti_gb2312"],
        "仿宋" | "仿宋_gb2312" | "fangsong" | "fangsong_gb2312" => &["仿宋", "仿宋_gb2312", "fangsong", "fangsong_gb2312"],
        "timesnewroman" => &["timesnewroman"],
        "arial" => &["arial"],
        "calibri" => &["calibri"],
        "微软雅黑" | "microsoftyahei" => &["微软雅黑", "microsoftyahei"],
        _ => NONE,
    }
}

/// 判断实际字体是否匹配期望字体集合（忽略大小写与别名）。
pub(crate) fn font_matches(actual: &str, expected_set: &[String]) -> bool {
    let a = norm_font(actual);
    let a_group: std::collections::HashSet<&str> = font_alias_group(&a).iter().copied().collect();
    for e in expected_set {
        let en = norm_font(e);
        if a == en || a_group.contains(en.as_str()) {
            return true;
        }
        // 前缀兼容：“SimSun-ExtB” 视为宋体系
        if (!en.is_empty()) && (a.starts_with(&en) || en.starts_with(&a)) {
            return true;
        }
    }
    false
}

/// 按 text_class 决定该 run 需要检查哪些字体槽位 → (中文字体?, 西文字体?)
fn classes_to_check(filter: &TextClassFilter, run_text: &str) -> (bool, bool) {
    let s = classify_text(run_text);
    match filter {
        TextClassFilter::Cjk => (s.has_cjk(), false),
        TextClassFilter::Latin => (false, s.latin_count > 0),
        TextClassFilter::Digit => (false, s.digit_count > 0),
        TextClassFilter::LatinOrDigit => (false, s.has_latin_or_digit()),
        TextClassFilter::All => (true, true),
        TextClassFilter::Auto => (s.has_cjk(), s.has_latin_or_digit()),
    }
}

pub struct RunFontFamilyChecker {
    pub text_class: TextClassFilter,
    pub expected_fonts: Vec<String>,
    pub blocked_fonts: Option<Vec<String>>,
}

impl Checker for RunFontFamilyChecker {
    fn rule_id(&self) -> &str {
        "run_font_family"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let mut out = Vec::new();
        let mut first_bad: Option<(usize, String, String)> = None; // (run_idx, 类别描述, 实际字体)
        let mut any_missing = false;
        for r in &p.runs {
            if r.text.trim().is_empty() {
                continue;
            }
            let (want_ea, want_ascii) = classes_to_check(&self.text_class, &r.text);
            if !want_ea && !want_ascii {
                continue;
            }
            let eff = effective_of(r);
            for (slot, label, active) in
                [(&eff.east_asian_font, "中文字体", want_ea), (&eff.ascii_font, "西文字体", want_ascii)]
            {
                if !active {
                    continue;
                }
                match slot {
                    None => any_missing = true,
                    Some(f) => {
                        let bad_expected = !self.expected_fonts.is_empty() && !font_matches(f, &self.expected_fonts);
                        let bad_blocked = self.blocked_fonts.as_ref().map(|b| font_matches(f, b)).unwrap_or(false);
                        if (bad_expected || bad_blocked) && first_bad.is_none() {
                            first_bad = Some((r.index, label.to_string(), f.clone()));
                        }
                    }
                }
            }
        }
        if let Some((idx, label, actual)) = first_bad {
            let excerpt = p
                .runs
                .iter()
                .find(|r| r.index == idx)
                .map(|r| excerpt_of(r.text.trim(), 30))
                .filter(|s| !s.is_empty());
            let expect = format!("{label}应为{}", self.expected_fonts.join(" 或 "));
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(p.global_index),
                    block_index: Some(p.block_index),
                    section_index: Some(p.section_index),
                    run_index: Some(idx),
                    page_hint: p.page_hint,
                    object_ref: None,
                },
                format!("{}，当前{label}为“{actual}”。", ctx.rule.message.trim_end_matches('。')),
                Some(expect),
                Some(format!("“{actual}”")),
                excerpt,
                0.85,
            ));
        } else if any_missing {
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(p.global_index),
                    block_index: Some(p.block_index),
                    section_index: Some(p.section_index),
                    ..Default::default()
                },
                "此处未能识别字体，建议人工检查。".into(),
                Some(excerpt_of(p.text.trim(), 30)).filter(|s| !s.is_empty()),
            ));
        }
        out
    }
}

// ---------------- font_allow_list ----------------

pub struct FontAllowListChecker {
    pub mode: crate::dsl::FontListMode,
    pub allowed_fonts: Option<Vec<String>>,
    pub blocked_fonts: Option<Vec<String>>,
    pub policy_blocklist: Vec<String>,
    pub policy_allowlist: Vec<String>,
}

impl Checker for FontAllowListChecker {
    fn rule_id(&self) -> &str {
        "font_allow_list"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        use crate::dsl::FontListMode;
        let mut out = Vec::new();
        let blocked_list = self.blocked_fonts.clone().unwrap_or_else(|| self.policy_blocklist.clone());
        let allow_list = self.allowed_fonts.clone().unwrap_or_else(|| self.policy_allowlist.clone());
        let mut hit: Option<(usize, String)> = None;
        for r in &p.runs {
            if r.text.trim().is_empty() {
                continue;
            }
            let eff = effective_of(r);
            let fonts: Vec<String> = [eff.east_asian_font.clone(), eff.ascii_font.clone()].into_iter().flatten().collect();
            for f in fonts {
                let bad = match self.mode {
                    FontListMode::Blocklist => font_matches(&f, &blocked_list),
                    FontListMode::Allowlist => !allow_list.is_empty() && !font_matches(&f, &allow_list),
                };
                if bad {
                    hit = Some((r.index, f));
                    break;
                }
            }
            if hit.is_some() {
                break;
            }
        }
        if let Some((idx, f)) = hit {
            // 面向普通用户：只提示“使用了装饰性字体”，不罗列内部字体映射细节。
            let mut v = violation_for(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(p.global_index),
                    block_index: Some(p.block_index),
                    section_index: Some(p.section_index),
                    run_index: Some(idx),
                    page_hint: p.page_hint,
                    object_ref: None,
                },
                match self.mode {
                    FontListMode::Blocklist => "此处使用了不建议的装饰性字体，学术论文建议使用常规字体。".to_string(),
                    FontListMode::Allowlist => "此处使用的字体不在常用学术字体范围内，建议确认学校要求。".to_string(),
                },
                Some("宋体、黑体、楷体、Times New Roman 等常规字体".into()),
                Some(format!("“{f}”")),
                Some(excerpt_of(p.text.trim(), 30)).filter(|s| !s.is_empty()),
                0.8,
            );
            if v.severity == Severity::Error {
                v.severity = Severity::Warning; // 字体偏好不是硬错误
            }
            out.push(v);
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dsl::{DirectParagraphProps, DirectRunProps, ParagraphRole, ParsedDocument};
    use crate::rules::tests_helper::config_from_json;
    use std::path::PathBuf;

    pub(crate) fn run_with(text: &str, idx: usize, size: Option<f64>, ea: Option<&str>, ascii: Option<&str>) -> crate::dsl::Run {
        crate::dsl::Run {
            index: idx,
            text: text.into(),
            direct: DirectRunProps {
                font_size_pt: size,
                ascii_font: ascii.map(String::from),
                east_asian_font: ea.map(String::from),
                ..Default::default()
            },
            character_style_id: None,
            effective: Some(EffectiveRunFormat {
                font_size_pt: size,
                ascii_font: ascii.map(String::from),
                east_asian_font: ea.map(String::from),
                ..Default::default()
            }),
            inline_objects: vec![],
            text_class_summary: classify_text(text),
        }
    }

    pub(crate) fn para(runs: Vec<crate::dsl::Run>, role: ParagraphRole) -> Paragraph {
        let text = runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("");
        Paragraph {
            global_index: 0,
            block_index: 0,
            section_index: 0,
            text,
            runs,
            direct: DirectParagraphProps::default(),
            style_id: None,
            effective: None,
            role,
            role_tags: vec![],
            role_confidence: 1.0,
            page_hint: Some(3),
            in_table: false,
            in_header_footer: false,
            in_footnote_endnote: false,
        }
    }

    pub(crate) fn empty_doc() -> ParsedDocument {
        ParsedDocument {
            path: PathBuf::from("/tmp/t.docx"),
            file_name: "t.docx".into(),
            sections: vec![],
            blocks: vec![],
            paragraphs: vec![],
            styles: Default::default(),
            doc_defaults: None,
            captions: vec![],
            citations: vec![],
            reference_entries: vec![],
            page_break_count: 0,
            parse_warnings: vec![],
        }
    }

    #[test]
    fn body_font_size_12_passes_105_fails_none_is_info() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-font-size").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        assert!(c.check_paragraph(&ctx, &para(vec![run_with("教育研究的发展。", 0, Some(12.0), None, None)], ParagraphRole::Body)).is_empty());
        let vs = c.check_paragraph(&ctx, &para(vec![run_with("教育研究的发展。", 0, Some(10.5), None, None)], ParagraphRole::Body));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Error);
        assert_eq!(vs[0].expected.as_deref(), Some("小四（12 pt）"));
        assert_eq!(vs[0].actual.as_deref(), Some("约 10.5 pt"));
        let vs = c.check_paragraph(&ctx, &para(vec![run_with("教育研究的发展。", 0, None, None, None)], ParagraphRole::Body));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
        assert!(vs[0].message.contains("人工检查"));
    }

    #[test]
    fn multiple_bad_runs_aggregate_to_one_violation() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-font-size").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let runs = vec![
            run_with("第一段文字", 0, Some(10.5), None, None),
            run_with("第二段文字", 1, Some(10.5), None, None),
            run_with("第三段文字", 2, Some(9.0), None, None),
        ];
        let vs = c.check_paragraph(&ctx, &para(runs, ParagraphRole::Body));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].location.run_index, Some(0));
        assert!(vs[0].excerpt.as_deref().unwrap().contains("第一段文字"));
    }

    #[test]
    fn heading1_allows_16_and_15_rejects_14() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "heading1-size").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let h = |sz: f64| para(vec![run_with("一、绪论", 0, Some(sz), None, None)], ParagraphRole::Heading { level: 1 });
        assert!(c.check_paragraph(&ctx, &h(16.0)).is_empty());
        assert!(c.check_paragraph(&ctx, &h(15.0)).is_empty());
        let vs = c.check_paragraph(&ctx, &h(14.0));
        assert_eq!(vs.len(), 1);
        assert!(vs[0].expected.as_deref().unwrap().contains("三号"));
    }

    #[test]
    fn cjk_font_simsun_passes_yahei_fails() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "body-cjk-font").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        assert!(c.check_paragraph(&ctx, &para(vec![run_with("教育公平研究", 0, Some(12.0), Some("宋体"), None)], ParagraphRole::Body)).is_empty());
        assert!(c.check_paragraph(&ctx, &para(vec![run_with("教育公平研究", 0, Some(12.0), Some("SimSun"), None)], ParagraphRole::Body)).is_empty());
        let vs = c.check_paragraph(&ctx, &para(vec![run_with("教育公平研究", 0, Some(12.0), Some("微软雅黑"), None)], ParagraphRole::Body));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Error);
        assert!(vs[0].message.contains("宋体"));
    }

    #[test]
    fn auto_class_checks_both_scripts_in_mixed_run() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "heading1-font").unwrap(); // text_class=auto, 期望黑体
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let ok = para(vec![run_with("一、AI 技术概述", 0, Some(16.0), Some("黑体"), Some("SimHei"))], ParagraphRole::Heading { level: 1 });
        assert!(c.check_paragraph(&ctx, &ok).is_empty());
        let bad = para(vec![run_with("一、AI 技术概述", 0, Some(16.0), Some("宋体"), Some("SimHei"))], ParagraphRole::Heading { level: 1 });
        let vs = c.check_paragraph(&ctx, &bad);
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("中文字体"));
    }

    #[test]
    fn decorative_font_blocklist_warns_without_internal_details() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "decorative-font-block").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let vs = c.check_paragraph(&ctx, &para(vec![run_with("封面标题", 0, Some(16.0), Some("华文彩云"), None)], ParagraphRole::Title));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].message.contains("装饰性字体"));
        assert!(!vs[0].message.contains("w:rFonts"));
    }

    #[test]
    fn font_alias_matching_case_insensitive() {
        assert!(font_matches("SIMSUN", &["宋体".into()]));
        assert!(font_matches("宋体", &["SimSun".into()]));
        assert!(font_matches("KaiTi_GB2312", &["楷体".into()]));
        assert!(!font_matches("微软雅黑", &["宋体".into(), "SimSun".into()]));
    }
}
