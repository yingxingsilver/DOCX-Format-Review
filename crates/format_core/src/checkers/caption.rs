//! caption —— 图表题注规则（文档级，一次扫描）。
//!
//! 判定原则：不凭“以图开头”就认定图题；优先使用解析器给出的 CaptionInfo
//! （已结合样式、正则与邻近对象），仅当解析器未提供时，才用规则 pattern +
//! 角色做低置信度兜底识别（此时一律降级 info/manual_review）。

use regex::Regex;

use crate::checkers::{excerpt_of, loc_of, violation_for};
use crate::dsl::{
    AuditContext, CaptionInfo, CaptionKind, CaptionNumberingRule, CaptionPosition, Checker,
    Location, Paragraph, ParagraphRole, Severity, Violation,
};

pub struct CaptionChecker {
    pub kind: CaptionKind,
    pub pattern: Regex,
    pub expected_alignment: Option<crate::dsl::Alignment>,
    pub expected_position: Option<CaptionPosition>,
    pub numbering: Option<CaptionNumberingRule>,
}

fn matches_kind(doc_kind: &CaptionKind, want: &CaptionKind) -> bool {
    doc_kind == want
}

/// 编号提取正则：进程级编译一次，跨线程只读共享（审查项：避免逐段重编译）。
fn number_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+(?:-\d+)*").expect("静态正则必然合法"))
}

/// 从题注文本提取编号序列（如 “1-1” → [1,1]，“3” → [3]）。
fn extract_numbers(text: &str) -> Vec<u32> {
    match number_re().find(text) {
        Some(m) => m
            .as_str()
            .split('-')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect(),
        None => Vec::new(),
    }
}

impl Checker for CaptionChecker {
    fn rule_id(&self) -> &str {
        "caption"
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        let want_role = match self.kind {
            CaptionKind::Figure => ParagraphRole::FigureCaption,
            CaptionKind::Table => ParagraphRole::TableCaption,
        };

        // 1) 解析器给出的高置信题注
        let parser_items: Vec<&CaptionInfo> = ctx
            .document
            .captions
            .iter()
            .filter(|c| matches_kind(&c.kind, &self.kind))
            .collect();
        // 2) 兜底：角色匹配 **或** pattern 命中（仅当解析器没有给出该类题注时）。
        //    两者取“或”是因为：解析器可能只标了角色未给 CaptionInfo，也可能样式不规范但文本以“图N”开头。
        //    兜底识别置信度低，产出的违规一律降级 info/manual_review。
        let fallback_paras: Vec<&Paragraph> = if parser_items.is_empty() {
            ctx.document
                .paragraphs
                .iter()
                .filter(|p| {
                    let t = p.text.trim();
                    if t.is_empty() {
                        return false;
                    }
                    p.role == want_role || self.pattern.is_match(t)
                })
                .collect()
        } else {
            Vec::new()
        };

        if parser_items.is_empty() && fallback_paras.is_empty() {
            // 论文可能本来就没有图/表 → 不报问题
            return out;
        }
        let low_confidence = parser_items.is_empty(); // 仅靠文本模式识别 → 置信度降低

        // 统一条目集合：(段落索引, 文本, 段落引用)
        let items: Vec<(usize, String, Option<&Paragraph>)> = if !parser_items.is_empty() {
            parser_items
                .iter()
                .map(|c| {
                    let p = ctx.document.paragraphs.get(c.paragraph_index);
                    (
                        c.paragraph_index,
                        p.map(|x| x.text.clone()).unwrap_or_default(),
                        p,
                    )
                })
                .collect()
        } else {
            fallback_paras
                .iter()
                .map(|p| (p.global_index, p.text.clone(), Some(*p)))
                .collect()
        };

        // ---- 对齐检查 ----
        if let Some(want_align) = self.expected_alignment {
            for (_idx, text, para) in &items {
                let Some(p) = para else { continue };
                let eff = p.effective.clone().unwrap_or_default();
                match eff.alignment.or(p.direct.alignment) {
                    None | Some(crate::dsl::Alignment::Unknown) => {}
                    Some(a) if a == want_align => {}
                    Some(a) => {
                        let mut v = violation_for(
                            ctx.rule,
                            &ctx.document.path,
                            loc_of(p),
                            format!(
                                "{}题注建议{}，当前为{}。",
                                kind_name_cn(&self.kind),
                                want_align.label_cn(),
                                a.label_cn()
                            ),
                            Some(want_align.label_cn().to_string()),
                            Some(a.label_cn().to_string()),
                            Some(excerpt_of(text.trim(), 30)),
                            0.85,
                        );
                        if low_confidence {
                            // 兜底识别只降低置信度，不改变规则配置的严重级别
                            // （对齐是段落上的确定事实，与“题注是否被认定”解耦）。
                            v.confidence = (v.confidence - 0.3).max(0.2);
                            v.requires_manual_review = true;
                        }
                        out.push(v);
                    }
                }
            }
        }

        // ---- 位置检查（figure 在图下方 / table 在表上方）----
        if let Some(want_pos) = &self.expected_position {
            if !parser_items.is_empty() {
                for c in &parser_items {
                    if c.position == CaptionPosition::Unknown
                        || c.nearby_object_block_index.is_none()
                    {
                        let mut v = violation_for(
                            ctx.rule,
                            &ctx.document.path,
                            Location {
                                paragraph_index: Some(c.paragraph_index),
                                ..Default::default()
                            },
                            format!(
                                "无法自动确认{}与{}的相对位置，请人工检查。",
                                kind_name_cn(&self.kind),
                                object_name_cn(&self.kind)
                            ),
                            Some(position_hint(want_pos)),
                            Some("未能判断".into()),
                            None,
                            0.4,
                        );
                        v.severity = Severity::Info;
                        v.requires_manual_review = true;
                        out.push(v);
                    } else if &c.position != want_pos {
                        let mut v = violation_for(
                            ctx.rule,
                            &ctx.document.path,
                            Location {
                                paragraph_index: Some(c.paragraph_index),
                                ..Default::default()
                            },
                            format!(
                                "{}应{}，当前位置不符。",
                                kind_name_cn(&self.kind),
                                position_hint(want_pos)
                            ),
                            Some(position_hint(want_pos)),
                            Some(position_hint(&c.position)),
                            None,
                            0.6,
                        );
                        v.severity = Severity::Warning;
                        v.requires_manual_review = true;
                        out.push(v);
                    }
                }
            } else {
                // 只有兜底识别 → 位置不可靠，统一一条 manual_review info
                let mut v = violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location::default(),
                    format!(
                        "{}的位置（相对{}）无法自动确认，请对照学校模板人工检查。",
                        kind_name_cn(&self.kind),
                        object_name_cn(&self.kind)
                    ),
                    Some(position_hint(want_pos)),
                    Some("未能判断".into()),
                    None,
                    0.3,
                );
                v.severity = Severity::Info;
                v.requires_manual_review = true;
                out.push(v);
            }
        }

        // ---- 编号连续性检查（一次遍历）----
        if let Some(num_rule) = &self.numbering {
            let mut seqs: Vec<(usize, Vec<u32>)> = items
                .iter()
                .map(|(idx, text, _)| (*idx, extract_numbers(text)))
                .filter(|(_, nums)| !nums.is_empty())
                .collect();
            seqs.sort_by_key(|(idx, _)| *idx);

            if let Some(first) = seqs.first() {
                let expect_start = num_rule.start;
                let actual_start = first.1.first().copied().unwrap_or(expect_start);
                if actual_start != expect_start {
                    let mut v = violation_for(
                        ctx.rule,
                        &ctx.document.path,
                        Location {
                            paragraph_index: Some(first.0),
                            ..Default::default()
                        },
                        format!(
                            "{}编号建议从 {expect_start} 开始，当前从 {actual_start} 开始。",
                            kind_name_cn(&self.kind)
                        ),
                        Some(format!("从 {expect_start} 开始")),
                        Some(format!("从 {actual_start} 开始")),
                        None,
                        0.7,
                    );
                    v.severity = Severity::Warning;
                    v.requires_manual_review = true;
                    out.push(v);
                }
            }

            let mut prev: Option<Vec<u32>> = None;
            let mut bad_spots: Vec<(Vec<u32>, Vec<u32>)> = Vec::new();
            for (_idx, nums) in &seqs {
                if let Some(pv) = &prev {
                    if !number_follows(pv, nums, num_rule) {
                        bad_spots.push((pv.clone(), nums.clone()));
                    }
                }
                prev = Some(nums.clone());
            }
            for (pv, cur) in bad_spots.into_iter().take(3) {
                let mut v = violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Location::default(),
                    format!(
                        "{}编号疑似跳号：从 {} 到了 {}，请核对。",
                        kind_name_cn(&self.kind),
                        join_num(&pv),
                        join_num(&cur)
                    ),
                    Some("编号连续".into()),
                    Some(format!("{} → {}", join_num(&pv), join_num(&cur))),
                    None,
                    0.6,
                );
                v.severity = Severity::Warning;
                v.requires_manual_review = true;
                out.push(v);
            }
        }
        out
    }
}

/// 编号是否合法衔接：同章节顺延 1；允许章节前缀时章节号可进位（如 1-2 → 2-1）。
/// 空切片保护（审查项 S3）：任一侧没有编号数字时无法判断，不算跳号；
/// 否则 `prev.len() - 1` 会在空切片上触发 usize 减法溢出 panic。
pub(crate) fn number_follows(prev: &[u32], cur: &[u32], rule: &CaptionNumberingRule) -> bool {
    if prev.is_empty() || cur.is_empty() {
        return true;
    }
    if prev.len() == cur.len() {
        let same_prefix = prev[..prev.len() - 1] == cur[..cur.len() - 1];
        let last_prev = *prev.last().expect("非空切片必有 last");
        let last_cur = *cur.last().expect("非空切片必有 last");
        if same_prefix {
            return last_cur == last_prev + 1;
        }
        // 前缀不同：若允许章节前缀，视为进入新章节（章号 +1、序号回到 1）
        if rule.allow_chapter_prefix && prev.len() >= 2 {
            let mut expect_prefix = prev[..prev.len() - 1].to_vec();
            let last = expect_prefix.len() - 1;
            expect_prefix[last] += 1;
            return expect_prefix == cur[..cur.len() - 1] && last_cur == 1;
        }
        return false;
    }
    // 长度变化（如 “图1” → “图2-1”）：无法可靠判断，不算跳号
    if rule.allow_chapter_prefix {
        return true;
    }
    !rule.continuous
}

fn join_num(nums: &[u32]) -> String {
    nums.iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("-")
}

fn kind_name_cn(k: &CaptionKind) -> &'static str {
    match k {
        CaptionKind::Figure => "图",
        CaptionKind::Table => "表",
    }
}

fn object_name_cn(k: &CaptionKind) -> &'static str {
    match k {
        CaptionKind::Figure => "图片",
        CaptionKind::Table => "表格",
    }
}

fn position_hint(p: &CaptionPosition) -> String {
    match p {
        CaptionPosition::AboveObject => "位于对象上方".into(),
        CaptionPosition::BelowObject => "位于对象下方".into(),
        CaptionPosition::Unknown => "位置待确认".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::font::tests::{empty_doc, para, run_with};
    use crate::dsl::{EffectiveParagraphFormat, ParsedDocument};
    use crate::rules::tests_helper::config_from_json;

    fn caption_doc(
        items: Vec<(&str, ParagraphRole, Option<crate::dsl::Alignment>)>,
        captions: Vec<CaptionInfo>,
    ) -> ParsedDocument {
        let mut d = empty_doc();
        d.paragraphs = items
            .into_iter()
            .enumerate()
            .map(|(i, (t, r, align))| {
                let mut p = para(vec![run_with(t, 0, Some(10.5), None, None)], r);
                p.global_index = i;
                p.block_index = i;
                p.effective = align.map(|a| EffectiveParagraphFormat {
                    alignment: Some(a),
                    ..Default::default()
                });
                p
            })
            .collect();
        d.captions = captions;
        d
    }

    #[test]
    fn figure_caption_centered_passes_left_warns() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = cfg.rules.iter().find(|r| r.id == "figure-caption").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = caption_doc(
            vec![(
                "图1 系统架构图",
                ParagraphRole::FigureCaption,
                Some(crate::dsl::Alignment::Center),
            )],
            vec![],
        );
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        assert!(
            c.check_document(&ctx)
                .iter()
                .filter(|v| v.message.contains("居中"))
                .count()
                == 0
        );
        let doc2 = caption_doc(
            vec![(
                "图1 系统架构图",
                ParagraphRole::FigureCaption,
                Some(crate::dsl::Alignment::Left),
            )],
            vec![],
        );
        let ctx2 = AuditContext {
            document: &doc2,
            config: &cfg,
            rule,
        };
        let vs = c.check_document(&ctx2);
        assert!(vs
            .iter()
            .any(|v| v.message.contains("居中") && v.severity == Severity::Warning));
    }

    #[test]
    fn figure_number_gap_warns() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = cfg.rules.iter().find(|r| r.id == "figure-caption").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = caption_doc(
            vec![
                (
                    "图1 架构图",
                    ParagraphRole::FigureCaption,
                    Some(crate::dsl::Alignment::Center),
                ),
                ("正文段落。", ParagraphRole::Body, None),
                (
                    "图3 流程图",
                    ParagraphRole::FigureCaption,
                    Some(crate::dsl::Alignment::Center),
                ),
            ],
            vec![],
        );
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        let vs = c.check_document(&ctx);
        assert!(vs.iter().any(|v| v.message.contains("跳号")
            && v.severity == Severity::Warning
            && v.requires_manual_review));
    }

    #[test]
    fn chapter_prefixed_numbers_are_allowed() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = cfg.rules.iter().find(|r| r.id == "table-caption").unwrap(); // allow_chapter_prefix=true
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let doc = caption_doc(
            vec![
                (
                    "表1-1 样本统计",
                    ParagraphRole::TableCaption,
                    Some(crate::dsl::Alignment::Center),
                ),
                (
                    "表1-2 回归结果",
                    ParagraphRole::TableCaption,
                    Some(crate::dsl::Alignment::Center),
                ),
            ],
            vec![],
        );
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        let vs = c.check_document(&ctx);
        assert!(
            !vs.iter().any(|v| v.message.contains("跳号")),
            "{:?}",
            vs.iter().map(|v| &v.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn unknown_position_degrades_to_manual_review_info() {
        let cfg = config_from_json(include_str!(
            "../../../../resources/rules/education_generic.json"
        ));
        let rule = cfg.rules.iter().find(|r| r.id == "figure-caption").unwrap();
        let c = crate::checkers::build_checker(rule, &cfg).unwrap();
        let cap = CaptionInfo {
            paragraph_index: 0,
            kind: CaptionKind::Figure,
            raw_label: "图1".into(),
            number_parts: vec![1],
            title_text: Some("架构图".into()),
            nearby_object_block_index: None,
            position: CaptionPosition::Unknown,
            confidence: 0.9,
        };
        let doc = caption_doc(
            vec![(
                "图1 架构图",
                ParagraphRole::FigureCaption,
                Some(crate::dsl::Alignment::Center),
            )],
            vec![cap],
        );
        let ctx = AuditContext {
            document: &doc,
            config: &cfg,
            rule,
        };
        let vs = c.check_document(&ctx);
        assert!(vs.iter().all(|v| v.severity != Severity::Error));
        assert!(vs.iter().any(|v| v.requires_manual_review));
    }

    #[test]
    fn extract_numbers_handles_chapter_prefix() {
        assert_eq!(extract_numbers("图1-2 系统架构"), vec![1, 2]);
        assert_eq!(extract_numbers("表3 结果"), vec![3]);
    }

    // ---- number_follows 边界单测（审查项 S3 / 补测计划第 3 条）----

    fn num_rule(continuous: bool, allow_chapter: bool) -> CaptionNumberingRule {
        CaptionNumberingRule {
            start: 1,
            continuous,
            allow_chapter_prefix: allow_chapter,
        }
    }

    #[test]
    fn number_follows_empty_slices_do_not_panic() {
        let rule = num_rule(true, false);
        // 空切片曾触发 usize 减法溢出 panic，修复后必须安全返回“不算跳号”。
        assert!(number_follows(&[], &[], &rule));
        assert!(number_follows(&[], &[1], &rule));
        assert!(number_follows(&[1], &[], &rule));
    }

    #[test]
    fn number_follows_same_length_sequences() {
        let rule = num_rule(true, false);
        assert!(number_follows(&[1], &[2], &rule), "同前缀顺延 1");
        assert!(!number_follows(&[1], &[3], &rule), "跳号应判不合法");
        assert!(number_follows(&[1, 2], &[1, 3], &rule), "章节内顺延");
        assert!(!number_follows(&[1, 2], &[2, 3], &rule), "前缀变化且不允许进位");
        let chapter = num_rule(false, true);
        assert!(
            number_follows(&[1, 2], &[2, 1], &chapter),
            "允许章节前缀时 1-2 → 2-1 视为进位"
        );
        assert!(
            !number_follows(&[1, 2], &[3, 1], &chapter),
            "章号 +2 不是合法进位"
        );
    }

    #[test]
    fn number_follows_length_change_is_not_skipped() {
        // 长度变化（如 “图1” → “图2-1”）：无法可靠判断，不算跳号。
        let rule = num_rule(true, false);
        assert!(number_follows(&[1], &[2, 1], &rule));
    }
}
