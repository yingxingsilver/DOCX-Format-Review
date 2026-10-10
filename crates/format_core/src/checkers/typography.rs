//! typography —— 文字排版类规则：punctuation / cjk_latin_spacing。
//!
//! 原则：不做语义理解，只做字符类判断；不确定的一律降级（info），参考文献/英文摘要等
//! 场景通过角色豁免与 exceptions 正则屏蔽，避免误报。

use crate::checkers::{excerpt_of, violation_for};
use crate::dsl::{
    is_cjk_ideograph, AuditContext, Checker, Location, Paragraph, ParagraphRole, PunctuationMode,
    Severity, SpacingMode, Violation,
};
use regex::Regex;

/// 半角标点 → 建议的全角标点（仅中文语境使用）。
const ASCII_PUNCT: &[(char, char)] = &[
    (',', '，'),
    (';', '；'),
    (':', '：'),
    ('?', '？'),
    ('!', '！'),
];

fn loc_of(p: &Paragraph) -> Location {
    Location {
        section_index: Some(p.section_index),
        block_index: Some(p.block_index),
        paragraph_index: Some(p.global_index),
        run_index: None,
        page_hint: p.page_hint,
        object_ref: None,
    }
}

/// 段落是否豁免半角标点检查：
/// - 参考文献条目本身大量使用西文标点；
/// - 英文摘要（Abstract）整段为拉丁文本；
/// - 页眉页脚/脚注尾注第一版不检查。
fn is_exempt_role(p: &Paragraph) -> bool {
    matches!(
        p.role,
        ParagraphRole::ReferenceEntry | ParagraphRole::ReferenceLabel | ParagraphRole::TableText
    ) || p.in_header_footer
        || p.in_footnote_endnote
}

/// 用 exceptions 正则命中的区间做屏蔽（URL、DOI、[J] 等）。
fn masked_ranges(text: &str, exceptions: &[Regex]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    for re in exceptions {
        for m in re.find_iter(text) {
            ranges.push((m.start(), m.end()));
        }
    }
    ranges
}

fn in_masked(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|(s, e)| pos >= *s && pos < *e)
}

// ---------------- punctuation ----------------

pub struct PunctuationChecker {
    pub mode: PunctuationMode,
    pub exception_regexes: Vec<Regex>,
}

impl PunctuationChecker {
    /// 统计一段“中文语境”文本中出现的半角标点（去重后按出现顺序）。
    fn ascii_punct_in_cjk(&self, text: &str) -> Vec<(char, char, usize)> {
        let masked = masked_ranges(text, &self.exception_regexes);
        let mut found: Vec<(char, char, usize)> = Vec::new();
        let mut seen: Vec<char> = Vec::new();
        for (i, c) in text.char_indices() {
            if in_masked(i, &masked) {
                continue;
            }
            // ConsistentOnly 模式第一版只报告最典型的逗号/分号混用。
            if matches!(self.mode, PunctuationMode::ConsistentOnly) && !matches!(c, ',' | ';') {
                continue;
            }
            if let Some(&(ascii, full)) = ASCII_PUNCT.iter().find(|(a, _)| *a == c) {
                if !seen.contains(&ascii) {
                    seen.push(ascii);
                    found.push((ascii, full, i));
                }
            }
        }
        found
    }
}

impl Checker for PunctuationChecker {
    fn rule_id(&self) -> &str {
        "punctuation"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        let text = p.text.trim();
        if text.is_empty() || is_exempt_role(p) {
            return Vec::new();
        }
        // 纯英文/数字段落不误报：要求存在 CJK 表意文字才进入检查。
        if !text.chars().any(is_cjk_ideograph) {
            return Vec::new();
        }
        // 英文摘要（Latin 为主的 AbstractBody）豁免。
        if p.role == ParagraphRole::AbstractBody {
            let cjk = text.chars().filter(|c| is_cjk_ideograph(*c)).count();
            let latin = text.chars().filter(|c| c.is_ascii_alphabetic()).count();
            if latin > cjk * 2 {
                return Vec::new();
            }
        }

        let found = self.ascii_punct_in_cjk(text);
        if found.is_empty() {
            return Vec::new();
        }
        let list = found
            .iter()
            .map(|(a, f, _)| format!("“{a}”应为“{f}”"))
            .collect::<Vec<_>>()
            .join("、");
        vec![violation_for(
            ctx.rule,
            &ctx.document.path,
            loc_of(p),
            format!("中文句子中出现了英文标点：{list}。"),
            Some("中文标点".into()),
            Some("英文标点".into()),
            Some(excerpt_of(text, 40)),
            0.85,
        )]
    }
}

// ---------------- cjk_latin_spacing ----------------

pub struct CjkLatinSpacingChecker {
    pub mode: SpacingMode,
}

impl CjkLatinSpacingChecker {
    /// 找到所有 “CJK↔英数直接相邻（无空格）” 的示例位置（最多取 3 个即可代表）。
    fn missing_spaces(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut out: Vec<String> = Vec::new();
        let is_latin_digit = |c: char| c.is_ascii_alphanumeric();
        let mut i = 1usize;
        while i < chars.len() && out.len() < 3 {
            let (prev, cur) = (chars[i - 1], chars[i]);
            let hit = (is_cjk_ideograph(prev) && is_latin_digit(cur))
                || (is_latin_digit(prev) && is_cjk_ideograph(cur));
            if hit {
                let s = chars[i - 1..=i].iter().collect::<String>();
                out.push(s);
            }
            i += 1;
        }
        out
    }
}

impl Checker for CjkLatinSpacingChecker {
    fn rule_id(&self) -> &str {
        "cjk_latin_spacing"
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, p: &Paragraph) -> Vec<Violation> {
        if is_exempt_role(p) {
            return Vec::new();
        }
        let hits = Self::missing_spaces(&p.text);
        if hits.is_empty() {
            return Vec::new();
        }
        let mut v = violation_for(
            ctx.rule,
            &ctx.document.path,
            loc_of(p),
            format!(
                "中文与英文/数字之间建议留一个半角空格，例如“{}”。",
                hits.join("”、“")
            ),
            Some("中英文之间有空格".into()),
            Some("未见空格".into()),
            Some(excerpt_of(p.text.trim(), 40)),
            0.6,
        );
        // recommend 模式只报 info；require/forbid 也不越过 warning。
        v.severity = match self.mode {
            SpacingMode::Recommend => Severity::Info,
            SpacingMode::Require | SpacingMode::Forbid => Severity::Warning,
        };
        vec![v]
    }
}

// =========================
// 单元测试
// =========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::build_checker;
    use crate::checkers::font::tests::{empty_doc, para, run_with};
    use crate::rules::tests_helper::config_from_json;

    fn punct_checker() -> (
        crate::dsl::RuleConfig,
        std::sync::Arc<crate::dsl::Rule>,
        crate::checkers::CompiledChecker,
    ) {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = std::sync::Arc::new(
            cfg.rules
                .iter()
                .find(|r| r.id == "punctuation-consistency")
                .unwrap()
                .clone(),
        );
        let c = build_checker(&rule, &cfg).unwrap();
        (cfg, rule, c)
    }

    #[test]
    fn ascii_comma_in_cjk_warns() {
        let (cfg, rule, c) = punct_checker();
        let doc = empty_doc();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        };
        let p = para(
            vec![run_with(
                "教育公平,是重要的议题。",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::Body,
        );
        let vs = c.check_paragraph(&ctx, &p);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].message.contains("英文标点"));
        assert!(vs[0].message.contains("，"));
    }

    #[test]
    fn pure_english_paragraph_not_flagged() {
        let (cfg, rule, c) = punct_checker();
        let doc = empty_doc();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        };
        let p = para(
            vec![run_with(
                "This is an English sentence, with commas.",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::Body,
        );
        assert!(c.check_paragraph(&ctx, &p).is_empty());
    }

    #[test]
    fn reference_entry_with_ascii_punct_not_flagged() {
        let (cfg, rule, c) = punct_checker();
        let doc = empty_doc();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        };
        let p = para(
            vec![run_with(
                "[1] 张三. 教育公平研究[J]. 教育研究, 2020, 41(3): 12-20.",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::ReferenceEntry,
        );
        assert!(c.check_paragraph(&ctx, &p).is_empty());
    }

    #[test]
    fn url_exception_masks_ascii_punct() {
        // 构造一条带 URL 例外的临时规则，验证 exceptions 屏蔽逻辑。
        let json = r#"{
            "version":"1","name":"t","defaults":{"tolerance":{}},
            "target_inference":{"figure_caption_regex":"图","table_caption_regex":"表","reference_entry_regex":"x","in_text_citation_regex":"y"},
            "rules":[{"id":"p","severity":"warning","message":"m","type":"punctuation","mode":"prefer_fullwidth_in_cjk",
                      "exceptions":["https?://\\S+"],
                      "target":{"roles":["body"]}}]
        }"#;
        let cfg = config_from_json(json);
        let rule = cfg.rules.first().unwrap();
        let c = build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        let p = para(
            vec![run_with(
                "详见 https://example.com/a,b,c 网站。",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::Body,
        );
        assert!(
            c.check_paragraph(&ctx, &p).is_empty(),
            "URL 内的英文逗号应被例外屏蔽"
        );
    }

    #[test]
    fn cjk_latin_space_recommend_is_info_only() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = cfg
            .rules
            .iter()
            .find(|r| r.id == "cjk-latin-space")
            .unwrap();
        // 该规则默认 enabled=false，这里直接手动构造 checker 验证行为。
        let c = build_checker(rule, &cfg).unwrap();
        let doc = empty_doc();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        let p = para(
            vec![run_with(
                "推进AI技术与教育融合。",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::Body,
        );
        let vs = c.check_paragraph(&ctx, &p);
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
        assert!(vs[0].message.contains("空格"));
        // 已含空格的文本不误报
        let ok = para(
            vec![run_with(
                "推进 AI 技术与教育融合。",
                0,
                Some(12.0),
                None,
                None,
            )],
            ParagraphRole::Body,
        );
        assert!(c.check_paragraph(&ctx, &ok).is_empty());
    }

    #[test]
    fn disabled_by_default_rule_not_built_in_shipped_config() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let built = crate::checkers::build_checkers(&cfg).unwrap();
        assert!(
            !built.iter().any(|c| c.rule_id() == "cjk-latin-space"),
            "cjk-latin-space 默认应禁用"
        );
    }
}
