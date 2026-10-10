//! page —— 页面级规则：page_size / page_margins / page_numbering。
//!
//! 输入单位已在解析阶段换算：页面尺寸 mm、页边距 cm。多节逐个检查；缺失数据一律降级 info。

use crate::checkers::{fmt_num, violation_for, within};
use crate::dsl::{
    AuditContext, Checker, Location, Margins, PageSize, PageNumberingExpectation, Severity, Violation,
};

pub struct PageSizeChecker {
    pub expected: PageSize,
}

impl Checker for PageSizeChecker {
    fn rule_id(&self) -> &str {
        "page_size"
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        if ctx.document.sections.is_empty() {
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location::default(),
                "未能识别页面尺寸，建议人工检查纸张设置。".into(),
                None,
            ));
            return out;
        }
        for sec in &ctx.document.sections {
            let Some(ps) = &sec.page_size else {
                out.push(crate::checkers::manual_hint(
                    ctx.rule,
                    &ctx.document.path,
                    Location { section_index: Some(sec.index), ..Default::default() },
                    format!("第 {} 节的页面尺寸未能识别，建议人工检查。", sec.index + 1),
                    None,
                ));
                continue;
            };
            let tol = ctx.config.tolerance().page_mm;
            let ok_w = within(ps.width_mm, self.expected.width_mm, tol);
            let ok_h = within(ps.height_mm, self.expected.height_mm, tol);
            if ok_w && ok_h {
                continue;
            }
            // Letter（216×279）等常见替代尺寸给更友好的文案。
            let is_letter = within(ps.width_mm, 215.9, tol) && within(ps.height_mm, 279.4, tol);
            let actual = format!("约 {} × {} mm", fmt_num(ps.width_mm), fmt_num(ps.height_mm));
            let message = if is_letter {
                "纸张不是 A4，当前接近 Letter（216 × 279 mm），请改为 A4。".to_string()
            } else {
                format!(
                    "纸张应为 {} × {} mm，当前为 {}。",
                    fmt_num(self.expected.width_mm),
                    fmt_num(self.expected.height_mm),
                    actual.replace("约 ", "")
                )
            };
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                Location { section_index: Some(sec.index), ..Default::default() },
                message,
                Some(format!("{} × {} mm", fmt_num(self.expected.width_mm), fmt_num(self.expected.height_mm))),
                Some(actual),
                None,
                0.95,
            ));
        }
        out
    }
}

pub struct PageMarginsChecker {
    pub expected: Margins,
}

impl PageMarginsChecker {
    /// 面向用户的方位词（“左侧/右侧/上侧/下侧”），比单字“左/右”更通顺。
    fn side_label(name: &str) -> String {
        if name.chars().next() == Some('首') || name.chars().next() == Some('尾') {
            format!("{name}页")
        } else {
            format!("{name}侧")
        }
    }

    fn one_side(name: &str, actual: f64, want: f64) -> (String, String, String) {
        let label = Self::side_label(name);
        (
            format!("页边距{label}应为 {} cm，当前约为 {} cm。", fmt_num(want), fmt_num(actual)),
            format!("{} cm", fmt_num(want)),
            format!("约 {} cm", fmt_num(actual)),
        )
    }
}

impl Checker for PageMarginsChecker {
    fn rule_id(&self) -> &str {
        "page_margins"
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        let tol = ctx.config.tolerance().margin_cm;
        for sec in &ctx.document.sections {
            let Some(m) = &sec.margins else {
                out.push(crate::checkers::manual_hint(
                    ctx.rule,
                    &ctx.document.path,
                    Location { section_index: Some(sec.index), ..Default::default() },
                    format!("第 {} 节的页边距未能识别，建议人工检查。", sec.index + 1),
                    None,
                ));
                continue;
            };
            let checks: [(&str, f64, Option<f64>); 4] = [
                ("上", m.top_cm, Some(self.expected.top_cm)),
                ("下", m.bottom_cm, Some(self.expected.bottom_cm)),
                ("左", m.left_cm, Some(self.expected.left_cm)),
                ("右", m.right_cm, Some(self.expected.right_cm)),
            ];
            for (name, actual, want) in checks {
                let Some(want) = want else { continue };
                if within(actual, want, tol) {
                    continue;
                }
                let (msg, exp, act) = Self::one_side(name, actual, want);
                out.push(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location { section_index: Some(sec.index), ..Default::default() },
                    msg,
                    Some(exp),
                    Some(act),
                    None,
                    0.95,
                ));
            }
        }
        out
    }
}

pub struct PageNumberingChecker {
    pub expected: PageNumberingExpectation,
}

impl Checker for PageNumberingChecker {
    fn rule_id(&self) -> &str {
        "page_numbering"
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        // 弱检查：正文起始节应重新编号且从期望值开始。
        // “正文节”取第一个包含 body/heading 段落的节；无法确定时整体降级 info。
        let body_section = ctx
            .document
            .paragraphs
            .iter()
            .find(|p| matches!(p.role, crate::dsl::ParagraphRole::Body | crate::dsl::ParagraphRole::Heading { .. }))
            .map(|p| p.section_index);
        let Some(sec_idx) = body_section else {
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location::default(),
                "未能定位正文起始位置，页码设置建议人工检查。".into(),
                None,
            ));
            return out;
        };
        let Some(sec) = ctx.document.sections.iter().find(|s| s.index == sec_idx) else {
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location { section_index: Some(sec_idx), ..Default::default() },
                "正文所在节的页面设置未能识别，页码建议人工检查。".into(),
                None,
            ));
            return out;
        };
        if self.expected.restart_at_body {
            if sec.restart_page_numbering != Some(true) {
                out.push(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location { section_index: Some(sec_idx), ..Default::default() },
                    "正文建议从第 1 页重新开始编页码，当前未检测到重新编号的设置。".to_string(),
                    Some(format!("正文起始至第 {} 页重新编号", self.expected.start)),
                    Some(if sec.restart_page_numbering == Some(false) { "未开启重新编号".into() } else { "未能识别".into() }),
                    None,
                    if sec.restart_page_numbering.is_some() { 0.8 } else { 0.4 },
                ));
            } else if sec.page_number_start.map(|v| v != self.expected.start).unwrap_or(false) {
                out.push(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location { section_index: Some(sec_idx), ..Default::default() },
                    format!("正文页码应从 {} 开始，当前从 {} 开始。", self.expected.start, sec.page_number_start.unwrap_or(0)),
                    Some(format!("从 {} 开始", self.expected.start)),
                    Some(format!("从 {} 开始", sec.page_number_start.unwrap_or(0))),
                    None,
                    0.8,
                ));
            }
        }
        // 页码位置（居中/右下）无法从 XML 稳定判断 → 固定附加一条 manual_review info。
        if !self.expected.allowed_positions.is_empty() {
            let mut v = violation_for(
                ctx.rule,
                &ctx.document.path,
                Location { section_index: Some(sec_idx), ..Default::default() },
                "页码位置（如居中或右下角）无法自动确认，请对照学校要求人工检查。".to_string(),
                Some("页码居中或右下角".into()),
                Some("需人工确认".into()),
                None,
                0.3,
            );
            v.severity = Severity::Info;
            v.requires_manual_review = true;
            out.push(v);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests_helper::config_from_json;
    use crate::dsl::{ParsedDocument, SectionProperties};
    use std::path::PathBuf;

    fn doc_with_section(page_size: Option<PageSize>, margins: Option<Margins>) -> ParsedDocument {
        ParsedDocument {
            path: PathBuf::from("/tmp/t.docx"),
            file_name: "t.docx".into(),
            sections: vec![SectionProperties {
                index: 0,
                page_size,
                margins,
                page_number_start: Some(1),
                restart_page_numbering: Some(true),
                title_page: None,
                header_references: vec![],
                footer_references: vec![],
            }],
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
    fn a4_passes_and_letter_fails() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "page-a4").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let a4 = doc_with_section(Some(PageSize { width_mm: 210.0, height_mm: 297.0 }), None);
        let ctx = AuditContext { document: &a4, config: &cfg, rule };
        assert!(c.check_document(&ctx).is_empty());
        let letter = doc_with_section(Some(PageSize { width_mm: 215.9, height_mm: 279.4 }), None);
        let ctx = AuditContext { document: &letter, config: &cfg, rule };
        let vs = c.check_document(&ctx);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Error);
        assert!(vs[0].message.contains("A4"));
    }

    #[test]
    fn margins_check_left_3cm() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "page-margins").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let good = doc_with_section(None, Some(Margins { top_cm: 2.5, bottom_cm: 2.5, left_cm: 3.0, right_cm: 2.5, header_cm: None, footer_cm: None, gutter_cm: None }));
        let ctx = AuditContext { document: &good, config: &cfg, rule };
        assert!(c.check_document(&ctx).is_empty());
        let bad = doc_with_section(None, Some(Margins { top_cm: 2.5, bottom_cm: 2.5, left_cm: 2.5, right_cm: 2.5, header_cm: None, footer_cm: None, gutter_cm: None }));
        let ctx = AuditContext { document: &bad, config: &cfg, rule };
        let vs = c.check_document(&ctx);
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("左侧"));
        assert_eq!(vs[0].expected.as_deref(), Some("3 cm"));
        assert_eq!(vs[0].actual.as_deref(), Some("约 2.5 cm"));
    }

    #[test]
    fn missing_page_size_degrades_to_info() {
        let cfg = config_from_json(include_str!("../../../../resources/rules/education_generic.json"));
        let rule = cfg.rules.iter().find(|r| r.id == "page-a4").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = doc_with_section(None, None);
        let ctx = AuditContext { document: &doc, config: &cfg, rule };
        let vs = c.check_document(&ctx);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
        assert!(vs[0].requires_manual_review);
    }
}
