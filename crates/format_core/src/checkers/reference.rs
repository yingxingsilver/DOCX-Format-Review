//! reference —— 文末参考文献检查（reference_list）。
//!
//! 第一版策略（刻意保守）：
//! - 只检查「编号是否从 start 开始连续」与「是否包含常见文献类型标识」两件事；
//! - 不做 GB/T 7714 全字段校验；缺类型标识一律 warning + 建议人工核对，绝不报 error。

use crate::checkers::violation_for;
use crate::dsl::{
    AuditContext, Checker, Location, ParagraphRole, ParsedDocument, Severity, Violation,
};
use regex::Regex;

/// 判断文本中是否出现任一类型标识（[J]、[M]、[EB/OL] 等）。
pub(crate) fn has_type_tag(text: &str, tags: &[String]) -> bool {
    let t = text.to_ascii_lowercase();
    tags.iter().any(|tag| t.contains(&tag.to_ascii_lowercase()))
}

/// 提取条目开头的编号，如 "[12] xxx" → Some(12)。
pub(crate) fn leading_number(re: &Regex, raw: &str) -> Option<u32> {
    re.captures(raw)
        .and_then(|c| c.get(0).map(|m| m.as_str().to_string()))
        .and_then(|s| {
            s.chars()
                .filter(|ch| ch.is_ascii_digit())
                .collect::<String>()
                .parse::<u32>()
                .ok()
        })
}

pub struct ReferenceListChecker {
    pub entry_pattern: Regex,
    pub required_type_tags: Vec<String>,
    pub check_continuous: bool,
    #[allow(dead_code)]
    pub check_cited: bool,
}

impl ReferenceListChecker {
    /// 收集参考文献条目：优先解析器结构化输出；为空时按角色 + 行首 [n] 兜底扫描一遍（O(n)）。
    fn collect_entries<'a>(&self, doc: &'a ParsedDocument) -> Vec<(usize, u32, String)> {
        if !doc.reference_entries.is_empty() {
            return doc
                .reference_entries
                .iter()
                .map(|e| (e.paragraph_index, e.number, e.raw.clone()))
                .collect();
        }
        doc.paragraphs
            .iter()
            .filter(|p| p.role == ParagraphRole::ReferenceEntry || p.role == ParagraphRole::Other)
            .filter_map(|p| {
                let n = leading_number(&self.entry_pattern, &p.text)?;
                Some((p.global_index, n, p.text.clone()))
            })
            .collect()
    }

    fn missing_numbers_msg(start: u32, missing: &[u32]) -> String {
        let list = missing
            .iter()
            .take(5)
            .map(|n| format!("[{n}]"))
            .collect::<Vec<_>>()
            .join("、");
        format!("参考文献编号建议从 [{start}] 开始连续，当前缺少 {list}。")
    }
}

impl Checker for ReferenceListChecker {
    fn rule_id(&self) -> &str {
        "reference_list"
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        let entries = self.collect_entries(ctx.document);
        if entries.is_empty() {
            // 未识别到参考文献条目：不报错，提示人工检查。
            out.push(crate::checkers::manual_hint(
                ctx.rule,
                &ctx.document.path,
                Location::default(),
                "未能识别出文末参考文献条目，建议人工检查。".into(),
                None,
            ));
            return out;
        }

        let start = entries.iter().map(|(_, n, _)| *n).min().unwrap_or(1);

        // 1) 连续性检查（一次排序扫描，O(n log n)，无 O(n²)）。
        if self.check_continuous {
            let mut nums: Vec<u32> = entries.iter().map(|(_, n, _)| *n).collect();
            nums.sort_unstable();
            nums.dedup();
            let mut missing: Vec<u32> = Vec::new();
            for expect in start..=*nums.last().unwrap() {
                if nums.binary_search(&expect).is_err() {
                    missing.push(expect);
                }
                if missing.len() >= 5 {
                    break;
                }
            }
            if !missing.is_empty() {
                out.push(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location {
                        paragraph_index: Some(entries.first().map(|e| e.0).unwrap_or(0)),
                        ..Default::default()
                    },
                    Self::missing_numbers_msg(start, &missing),
                    Some(format!("从 [{start}] 开始连续编号")),
                    Some(format!("共 {} 条，存在缺号", entries.len())),
                    None,
                    0.8,
                ));
            }
        }

        // 2) 类型标识检查：缺标识的条目聚合为一条 warning + 人工复核。
        let no_tag: Vec<&(usize, u32, String)> = entries
            .iter()
            .filter(|(_, _, raw)| !has_type_tag(raw, &self.required_type_tags))
            .collect();
        if !no_tag.is_empty() {
            let excerpt = crate::checkers::excerpt_of(&no_tag[0].2, 40);
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                Location {
                    paragraph_index: Some(no_tag[0].0),
                    page_hint: ctx
                        .document
                        .paragraphs
                        .get(no_tag[0].0)
                        .and_then(|p| p.page_hint),
                    ..Default::default()
                },
                format!(
                    "有 {} 条参考文献未包含常见文献类型标识（如 {}），建议人工核对。",
                    no_tag.len(),
                    self.required_type_tags.join("、")
                ),
                Some(self.required_type_tags.join("、")),
                Some("未见类型标识".into()),
                Some(excerpt),
                0.6,
            ));
            for v in out.last_mut() {
                v.severity = Severity::Warning;
                v.requires_manual_review = true;
            }
        }
        out
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
    use crate::dsl::ReferenceEntry;
    use crate::rules::tests_helper::config_from_json;

    fn rule_ctx(
        doc: &ParsedDocument,
    ) -> (crate::dsl::RuleConfig, std::sync::Arc<crate::dsl::Rule>) {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = std::sync::Arc::new(
            cfg.rules
                .iter()
                .find(|r| r.id == "reference-list")
                .unwrap()
                .clone(),
        );
        (cfg, rule)
    }

    fn entry(idx: usize, number: u32, raw: &str) -> ReferenceEntry {
        ReferenceEntry {
            paragraph_index: idx,
            number,
            raw: raw.into(),
            type_tag: None,
            has_year: true,
            has_pages: false,
            has_journal_or_publisher: false,
        }
    }

    #[test]
    fn continuous_123_passes_and_13_flags_missing_2() {
        let mut doc = empty_doc();
        doc.reference_entries = vec![
            entry(0, 1, "[1] 张三. 教育公平研究[J]. 教育研究, 2020."),
            entry(1, 2, "[2] 李四. 课程论[M]. 北京: 教育出版社, 2018."),
            entry(2, 3, "[3] 王五. 论文写作[D]. 华东师范大学, 2021."),
        ];
        let (cfg, rule) = rule_ctx(&doc);
        let c = build_checker(&rule, &cfg).unwrap();
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        };
        assert!(c.check_document(&ctx).is_empty(), "连续且含类型标识应通过");

        let mut doc2 = empty_doc();
        doc2.reference_entries = vec![
            entry(0, 1, "[1] 张三. 研究[J]. 期刊, 2020."),
            entry(1, 3, "[3] 李四. 专著[M]. 出版社, 2018."),
        ];
        let vs = c.check_document(&AuditContext {
            document: &doc2,
            config: &cfg,
            rule: &rule,
        });
        assert!(vs
            .iter()
            .any(|v| v.message.contains("缺少 [2]") && v.severity == Severity::Warning));
    }

    #[test]
    fn missing_type_tag_is_warning_with_manual_review() {
        let mut doc = empty_doc();
        doc.reference_entries = vec![entry(0, 1, "[1] 张三. 某本没有类型标识的书. 2020.")];
        let (cfg, rule) = rule_ctx(&doc);
        let c = build_checker(&rule, &cfg).unwrap();
        let vs = c.check_document(&AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        });
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].requires_manual_review);
        assert!(vs[0].message.contains("类型标识"));
    }

    #[test]
    fn fallback_scans_reference_role_paragraphs() {
        let mut doc = empty_doc();
        doc.paragraphs = vec![para(
            vec![run_with(
                "[1] 赵六. 实证分析[J]. 统计研究, 2019.",
                0,
                Some(10.5),
                None,
                None,
            )],
            ParagraphRole::ReferenceEntry,
        )];
        let (cfg, rule) = rule_ctx(&doc);
        let c = build_checker(&rule, &cfg).unwrap();
        let vs = c.check_document(&AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        });
        // 单条 [1] 连续、含 [J]，应无违规。
        assert!(
            vs.is_empty(),
            "{:?}",
            vs.iter().map(|v| &v.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn empty_reference_section_degrades_to_info() {
        let doc = empty_doc();
        let (cfg, rule) = rule_ctx(&doc);
        let c = build_checker(&rule, &cfg).unwrap();
        let vs = c.check_document(&AuditContext {
            document: &doc,
            config: &cfg,
            rule: &rule,
        });
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Info);
        assert!(vs[0].message.contains("人工检查"));
    }

    #[test]
    fn leading_number_and_type_tag_helpers() {
        let re = Regex::new(r"^\s*\[\d+\]").unwrap();
        assert_eq!(leading_number(&re, "  [12] 文献标题"), Some(12));
        assert_eq!(leading_number(&re, "无编号条目"), None);
        assert!(has_type_tag(
            "某某研究[J]. 期刊",
            &["[J]".to_string(), "[M]".to_string()]
        ));
        assert!(has_type_tag(
            "Online source [EB/OL]",
            &["[EB/OL]".to_string()]
        ));
        assert!(!has_type_tag("没有任何标识", &["[J]".to_string()]));
    }
}
