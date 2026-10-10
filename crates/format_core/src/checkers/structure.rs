//! structure —— 文档结构类检查器：sequence / heading_numbering / citation_order。
//!
//! 设计约定：
//! - 三者均为 document 级，单遍 O(n) 扫描全部段落，避免 O(n²)。
//! - 结构差异在不同学校间很大，因此一律不报 error：最高只到 warning，
//!   且凡是需要人工确认语义的结论都设置 manual_review = true。
//! - 正则已在规则加载期预编译（见 checkers::build_checker），这里只做匹配。

use regex::Regex;

use crate::checkers::{excerpt_of, loc_of, violation_for};
use crate::dsl::{
    AuditContext, Checker, CitationOccurrence, Paragraph, ParagraphRole, SequenceAnchor, Severity,
    Violation,
};

/// 锚点是否命中某段落：labels 前缀/包含、roles、regex 任一命中即算。
fn anchor_hits(a: &SequenceAnchor, p: &Paragraph, re: Option<&Regex>) -> bool {
    if a
        .labels
        .iter()
        .any(|l| !l.is_empty() && (p.text.starts_with(l.as_str()) || p.text.contains(l.as_str())))
    {
        return true;
    }
    if a.roles.iter().any(|r| match ParagraphRole::from_kind_str(r) {
        Some(ParagraphRole::Heading { .. }) => p.role.heading_level().is_some(),
        Some(other) => p.role == other,
        None => false,
    }) {
        return true;
    }
    match re {
        Some(re) => re.is_match(&p.text),
        None => false,
    }
}

/// 统一降级：warning + 需要人工复核（结构类问题永不报 error）。
fn with_mr(mut v: Violation) -> Violation {
    v.severity = Severity::Warning;
    v.requires_manual_review = true;
    v
}

// =========================
// sequence
// =========================

/// sequence：检查“摘要 → 关键词 → 正文 → 参考文献”等锚点的出现顺序。
pub struct SequenceChecker {
    pub anchors: Vec<SequenceAnchor>,
    pub strict: bool,
    /// 与 anchors 一一对应；None 表示该锚点无 regex。
    pub anchor_regexes: Vec<Option<Regex>>,
}

/// 锚点面向用户的中文名称：优先 labels 首个，其次按已知 id 映射，最后回退原 id。
fn anchor_display_name(a: &SequenceAnchor) -> String {
    if let Some(first) = a.labels.first() {
        return first.trim_end_matches(['：', ':']).to_string();
    }
    match a.id.as_str() {
        "title" => "标题",
        "author" => "作者",
        "affiliation" => "作者单位",
        "abstract" | "abstract_cn" => "摘要",
        "abstract_en" => "英文摘要",
        "keywords" | "keywords_cn" => "关键词",
        "keywords_en" => "英文关键词",
        "toc" => "目录",
        "body" => "正文",
        "references" => "参考文献",
        "acknowledgements" => "致谢",
        _ => return a.id.clone(),
    }
    .to_string()
}

impl Checker for SequenceChecker {
    fn rule_id(&self) -> &str {
        ""
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        // 一次扫描记录每个锚点首次命中的全局段落号。
        let mut first_at: Vec<Option<usize>> = vec![None; self.anchors.len()];
        for p in &ctx.document.paragraphs {
            for (i, a) in self.anchors.iter().enumerate() {
                if first_at[i].is_none()
                    && anchor_hits(a, p, self.anchor_regexes.get(i).and_then(|x| x.as_ref()))
                {
                    first_at[i] = Some(p.global_index);
                }
            }
        }
        // 必要锚点缺失 → warning + manual_review（不同学校允许缺作者/单位等）。
        let mut present: Vec<(usize, usize)> = Vec::new();
        for (i, at) in first_at.iter().enumerate() {
            match at {
                Some(idx) => present.push((i, *idx)),
                None => {
                    if !self.anchors[i].optional {
                        out.push(with_mr(violation_for(
                            ctx.rule,
                            &ctx.document.path,
                            Default::default(),
                            format!(
                                "没有识别到“{}”部分，请人工确认论文结构。",
                                anchor_display_name(&self.anchors[i])
                            ),
                            Some("包含全部必需章节".into()),
                            Some(format!("缺少 {}", self.anchors[i].id)),
                            None,
                            0.4,
                        )));
                    }
                }
            }
        }
        // 相对顺序检查：仅比较相邻的已命中锚点。
        for w in present.windows(2) {
            let (pos1, pos2) = (w[0].1, w[1].1);
            if pos1 > pos2 {
                let name1 = self.anchors[w[0].0].id.clone();
                let name2 = self.anchors[w[1].0].id.clone();
                let loc = ctx
                    .document
                    .paragraphs
                    .iter()
                    .find(|p| p.global_index == pos2)
                    .map(loc_of)
                    .unwrap_or_default();
                out.push(with_mr(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    loc,
                    format!("论文结构顺序建议调整：“{name2}”出现在“{name1}”之前。"),
                    Some(format!("{name1} 在 {name2} 之前")),
                    Some(format!("{name2} 在前")),
                    None,
                    0.5,
                )));
                if !self.strict {
                    break;
                }
            }
        }
        out
    }
}

// =========================
// heading_numbering
// =========================

/// 中文数字（一~九十九）→ 数值。
fn cn_num_to_u32(s: &str) -> Option<u32> {
    const DIGITS: [(&str, u32); 9] = [
        ("一", 1), ("二", 2), ("三", 3), ("四", 4), ("五", 5),
        ("六", 6), ("七", 7), ("八", 8), ("九", 9),
    ];
    let mut ten: u32 = 0;
    let mut unit: Option<u32> = None;
    for c in s.chars() {
        if c == '十' {
            ten = unit.unwrap_or(1) * 10;
            unit = None;
        } else if let Some((_, v)) = DIGITS.iter().find(|(d, _)| d.chars().next() == Some(c)) {
            unit = Some(*v);
        } else {
            return None;
        }
    }
    match unit {
        Some(u) if ten == 0 => Some(u),
        Some(u) => Some(ten + u),
        None if ten > 0 => Some(ten),
        None => None,
    }
}

/// heading_numbering：识别标题编号体系并检查混用/层级错配。
/// 只做正则与层级模式匹配，不理解语义。
pub struct HeadingNumberingChecker {
    pub allowed_schemes: Vec<String>,
    pub require_consistent: bool,
    pub check_level_match: bool,
    /// 构建期编译一次（审查项：避免每段落 thread_local 惰性重编译与 TLS 开销）。
    pub cn_res: [Regex; 4],
    pub arabic_res: [Regex; 3],
}

/// 兜底正则（规则未显式提供时使用）；非法 pattern 一律回退到默认值，不 panic。
pub fn heading_default_regexes() -> ([Regex; 4], [Regex; 3]) {
    let cn = [
        Regex::new(r"^([一二三四五六七八九十]{1,3})、").unwrap(),
        Regex::new(r"^（([一二三四五六七八九十]{1,3})）").unwrap(),
        Regex::new(r"^(\d{1,3})[\.、]").unwrap(),
        Regex::new(r"^（(\d{1,3})）").unwrap(),
    ];
    let ar = [
        Regex::new(r"^(\d{1,2}(?:\.\d{1,2}){2})(?:\.)?[\s、.]").unwrap(),
        // 二级“1.1”后可能直接跟中文标题文字：分隔符可选，再用捕获组首字符类别复核（regex crate 不支持前瞻）。
        Regex::new(r"^(\d{1,2}\.\d{1,2})(?:\.)?([\s、.]|[\u{4e00}-\u{9fff}]?)").unwrap(),
        Regex::new(r"^(\d{1,2})(?:\.)?[\s、.]").unwrap(),
    ];
    (cn, ar)
}

impl Checker for HeadingNumberingChecker {
    fn rule_id(&self) -> &str {
        ""
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut cn_counts = [0usize; 4]; // 一、 / （一） / 1. / （1）
        let mut arabic_counts = [0usize; 3]; // 1 / 1.1 / 1.1.1
        let mut level_mismatch = 0usize;
        let mut mismatch_excerpt: Option<String> = None;

        for p in &ctx.document.paragraphs {
            let Some(level) = p.role.heading_level() else { continue };
            let text = p.text.trim_start();
            let res = &self.cn_res;
            // 先尝试“带小数点”的阿拉伯层级编号（1.1 / 1.1.1），它不属于 cn_mixed；
            // 再按 cn_mixed 四档（一、 / （一） / 1. / （1））分类；
            // 最后才是纯数字开头的 arabic 一级（"1 引言"）。
            let mut matched_arabic: Option<(usize, String)> = None;
            if let Some(cap) = self.arabic_res[0].captures(text) {
                matched_arabic = Some((1usize, cap[1].to_string()));
            } else if let Some(cap) = self.arabic_res[1].captures(text) {
                // 二级模式第 2 捕获组须为分隔符/汉字/行尾，避免 "2.1.3" 误判为二级。
                let ok = match cap.get(2).map(|m| m.as_str()) {
                    None | Some("") => cap[0].ends_with(&cap[1]),
                    Some(ch) => ch.chars().next().is_some_and(|c| {
                        c.is_whitespace() || c == '、' || c == '.' || crate::dsl::is_cjk_ideograph(c)
                    }),
                };
                if ok {
                    matched_arabic = Some((2usize, cap[1].to_string()));
                }
            }
            if let Some((inferred, num)) = matched_arabic.clone() {
                arabic_counts[inferred.min(3) - 1] += 1;
                if self.check_level_match && inferred != level as usize && num.split('.').count() == inferred {
                    level_mismatch += 1;
                    if mismatch_excerpt.is_none() {
                        mismatch_excerpt = Some(excerpt_of(&p.text, 30));
                    }
                }
                continue;
            }
            let matched_cn = if res[0].captures(text).is_some_and(|c| cn_num_to_u32(&c[1]).is_some()) {
                Some(1usize)
            } else if res[1].captures(text).is_some_and(|c| cn_num_to_u32(&c[1]).is_some()) {
                Some(2)
            } else if res[2].captures(text).is_some() {
                Some(3)
            } else if res[3].captures(text).is_some() {
                Some(4)
            } else {
                None
            };
            if let Some(slot) = matched_cn {
                cn_counts[slot - 1] += 1;
                continue;
            }
            // 说明：纯数字一级编号（“1”“1.”）与 cn_mixed 第三档（“1. 2. 3.”）形态相同，
            // 无法在不看全文的情况下区分；上文已把带小数点的层级编号归入 arabic_decimal，
            // 这里将裸 “N./N、” 归入 cn_mixed 计数，混用判定仍可通过其它档位发现。
            if let Some((inferred, num)) = matched_arabic {
                arabic_counts[inferred - 1] += 1;
                if self.check_level_match
                    && inferred != level as usize
                    && num.split('.').count() == inferred
                {
                    level_mismatch += 1;
                    if mismatch_excerpt.is_none() {
                        mismatch_excerpt = Some(excerpt_of(&p.text, 30));
                    }
                }
            }
        }

        let cn_total: usize = cn_counts.iter().sum();
        let arabic_total: usize = arabic_counts.iter().sum();
        let mut out = Vec::new();
        if self.require_consistent && cn_total > 0 && arabic_total > 0 {
            out.push(with_mr(violation_for(
                ctx.rule,
                &ctx.document.path,
                Default::default(),
                "标题编号混用了两种体系（如“一、”与“1.1”），建议全文统一为一种。".into(),
                Some("单一编号体系".into()),
                Some(format!("中文体系 {cn_total} 处、数字体系 {arabic_total} 处")),
                None,
                0.6,
            )));
        }
        if self.check_level_match && level_mismatch > 0 {
            out.push(violation_for(
                ctx.rule,
                &ctx.document.path,
                Default::default(),
                format!("有 {level_mismatch} 个标题的编号层级与标题级别不一致，建议人工核对。"),
                Some("编号层级与标题级别一致".into()),
                Some("存在不一致".into()),
                mismatch_excerpt,
                0.5,
            ));
        }
        out
    }
}

// =========================
// citation_order
// =========================

/// citation_order：正文 [n] 引用与文末参考文献对照。
/// 正则可能误匹配，因此所有结论一律 warning + manual_review。
pub struct CitationOrderChecker {
    pub require_ascending: bool,
    pub allow_repeated: bool,
}

impl Checker for CitationOrderChecker {
    fn rule_id(&self) -> &str {
        ""
    }

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let mut out = Vec::new();
        let listed: Vec<u32> = ctx.document.reference_entries.iter().map(|e| e.number).collect();

        // 文末编号连续性（应从 1 开始）。
        if !listed.is_empty() {
            let mut sorted = listed.clone();
            sorted.sort_unstable();
            sorted.dedup();
            let mut missing: Vec<u32> = Vec::new();
            if sorted[0] != 1 {
                missing.push(1);
            }
            for w in sorted.windows(2) {
                if w[1] > w[0] + 1 {
                    missing.extend((w[0] + 1)..w[1]);
                }
            }
            if !missing.is_empty() {
                let shown: Vec<String> = missing.iter().take(5).map(|n| format!("[{n}]")).collect();
                out.push(with_mr(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Default::default(),
                    format!("参考文献编号建议从 [1] 开始连续，当前缺少 {}。", shown.join("、")),
                    Some("编号连续".into()),
                    Some(format!("缺少 {}", shown.join("、"))),
                    None,
                    0.6,
                )));
            }
        }

        // 正文引用序列。
        let cited_seq: Vec<u32> =
            ctx.document.citations.iter().flat_map(|c| c.numbers.iter().copied()).collect();
        if cited_seq.is_empty() || listed.is_empty() {
            return out;
        }
        let cited_set: std::collections::HashSet<u32> = cited_seq.iter().copied().collect();
        let listed_set: std::collections::HashSet<u32> = listed.iter().copied().collect();

        let mut dangling: Vec<u32> = cited_set.difference(&listed_set).copied().collect();
        dangling.sort_unstable();
        if !dangling.is_empty() {
            let shown: Vec<String> = dangling.iter().take(5).map(|n| format!("[{n}]")).collect();
            out.push(with_mr(violation_for(
                ctx.rule,
                &ctx.document.path,
                Default::default(),
                format!("正文引用了 {}，但文末参考文献中没有对应条目。", shown.join("、")),
                Some("每条引用都有对应文献".into()),
                Some(format!("{} 未列出", shown.join("、"))),
                None,
                0.5,
            )));
        }
        let mut uncited: Vec<u32> = listed_set.difference(&cited_set).copied().collect();
        uncited.sort_unstable();
        if !uncited.is_empty() {
            let shown: Vec<String> = uncited.iter().take(5).map(|n| format!("[{n}]")).collect();
            out.push(with_mr(violation_for(
                ctx.rule,
                &ctx.document.path,
                Default::default(),
                format!("文末列出了 {}，但正文从未引用。", shown.join("、")),
                Some("所列文献均在正文中被引用".into()),
                Some(format!("{} 未被引用", shown.join("、"))),
                None,
                0.5,
            )));
        }

        // 递增性：统计“回跳”次数（紧邻重复引用可豁免），超过 2 处才提示，避免误伤。
        if self.require_ascending {
            let mut regressions = 0usize;
            let mut prev = 0u32;
            for n in &cited_seq {
                if *n < prev && !(self.allow_repeated && *n == prev) {
                    regressions += 1;
                }
                prev = *n;
            }
            if regressions > 2 {
                out.push(with_mr(violation_for(
                    ctx.rule,
                    &ctx.document.path,
                    Default::default(),
                    "正文引用编号顺序有明显回跳，建议按出现先后重新编号。".into(),
                    Some("引用编号大体递增".into()),
                    Some(format!("检测到 {regressions} 处回跳")),
                    None,
                    0.4,
                )));
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
    use crate::checkers::font::tests::{empty_doc, para, run_with};
    use crate::dsl::{DirectParagraphProps, ReferenceEntry, RuleConfig};

    fn test_rule() -> RuleOwned {
        RuleOwned(
            crate::rules::tests_helper::config_from_json(
                r#"{"name":"t","rules":[{"id":"structure-order","severity":"warning","message":"结构顺序问题。","manual_review":true,"type":"sequence","anchors":[]}]}"#,
            )
            .rules
            .into_iter()
            .next()
            .unwrap(),
        )
    }
    struct RuleOwned(crate::dsl::Rule);
    impl std::ops::Deref for RuleOwned {
        type Target = crate::dsl::Rule;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    fn ctx<'a>(doc: &'a crate::dsl::ParsedDocument, rule: &'a crate::dsl::Rule) -> AuditContext<'a> {
        let cfg: &'static RuleConfig = Box::leak(Box::new(
            crate::rules::tests_helper::config_from_json(r#"{"name":"t","rules":[]}"#),
        ));
        AuditContext { document: doc, config: cfg, rule }
    }

    fn set_idx(doc: &mut crate::dsl::ParsedDocument) {
        for (i, p) in doc.paragraphs.iter_mut().enumerate() {
            p.global_index = i;
        }
    }

    fn abs_para(text: &str, role: ParagraphRole) -> Paragraph {
        let mut p = para(vec![run_with(text, 0, None, None, None)], role);
        p.text = text.into();
        p.direct = DirectParagraphProps::default();
        p
    }

    #[test]
    fn sequence_ok_when_abstract_before_keywords() {
        let r = test_rule();
        let c = SequenceChecker {
            anchors: vec![
                SequenceAnchor { id: "摘要".into(), labels: vec!["摘要".into()], roles: vec![], regex: None, optional: false },
                SequenceAnchor { id: "关键词".into(), labels: vec!["关键词".into()], roles: vec![], regex: None, optional: false },
            ],
            strict: false,
            anchor_regexes: vec![None, None],
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![
            abs_para("摘要：本文研究了格式审核方法。", ParagraphRole::AbstractBody),
            abs_para("关键词：格式；审核；DOCX", ParagraphRole::KeywordsBody),
        ];
        set_idx(&mut doc);
        assert!(c.check_document(&ctx(&doc, &r)).is_empty());
    }

    #[test]
    fn sequence_flags_reversed_order_as_warning_manual() {
        let r = test_rule();
        let c = SequenceChecker {
            anchors: vec![
                SequenceAnchor { id: "摘要".into(), labels: vec!["摘要".into()], roles: vec![], regex: None, optional: false },
                SequenceAnchor { id: "关键词".into(), labels: vec!["关键词".into()], roles: vec![], regex: None, optional: false },
            ],
            strict: false,
            anchor_regexes: vec![None, None],
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![
            abs_para("关键词：格式；审核", ParagraphRole::KeywordsBody),
            abs_para("摘要：本文研究了格式审核方法。", ParagraphRole::AbstractBody),
        ];
        set_idx(&mut doc);
        let vs = c.check_document(&ctx(&doc, &r));
        assert_eq!(vs.len(), 1);
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].requires_manual_review);
        assert!(vs[0].message.contains("摘要"));
    }

    #[test]
    fn sequence_optional_anchor_missing_is_ignored() {
        let r = test_rule();
        let c = SequenceChecker {
            anchors: vec![
                SequenceAnchor { id: "摘要".into(), labels: vec!["摘要".into()], roles: vec![], regex: None, optional: false },
                SequenceAnchor { id: "作者".into(), labels: vec![], roles: vec!["author".into()], regex: None, optional: true },
            ],
            strict: false,
            anchor_regexes: vec![None, None],
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![abs_para("摘要：内容", ParagraphRole::AbstractBody)];
        set_idx(&mut doc);
        assert!(c.check_document(&ctx(&doc, &r)).is_empty(), "optional 锚点缺失不应报错");
    }

    #[test]
    fn sequence_required_anchor_missing_warns() {
        let r = test_rule();
        let c = SequenceChecker {
            anchors: vec![
                SequenceAnchor { id: "摘要".into(), labels: vec!["摘要".into()], roles: vec![], regex: None, optional: false },
                SequenceAnchor { id: "参考文献".into(), labels: vec!["参考文献".into()], roles: vec![], regex: None, optional: false },
            ],
            strict: false,
            anchor_regexes: vec![None, None],
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![abs_para("摘要：内容", ParagraphRole::AbstractBody)];
        set_idx(&mut doc);
        let vs = c.check_document(&ctx(&doc, &r));
        assert_eq!(vs.len(), 1);
        assert!(vs[0].message.contains("参考文献"));
        assert!(vs[0].requires_manual_review);
    }

    #[test]
    fn heading_scheme_detects_cn_and_arabic_mix() {
        let r = test_rule();
        let (cn_res, arabic_res) = heading_default_regexes();
        let c = HeadingNumberingChecker {
            allowed_schemes: vec!["cn_mixed".into(), "arabic_decimal".into()],
            require_consistent: true,
            check_level_match: false,
            cn_res,
            arabic_res,
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![
            abs_para("一、引言", ParagraphRole::Heading { level: 1 }),
            abs_para("2.1 方法", ParagraphRole::Heading { level: 2 }),
        ];
        set_idx(&mut doc);
        let vs = c.check_document(&ctx(&doc, &r));
        assert_eq!(vs.len(), 1, "混用应报一条");
        assert_eq!(vs[0].severity, Severity::Warning);
        assert!(vs[0].requires_manual_review);
        assert!(vs[0].message.contains("混用"));
    }

    #[test]
    fn heading_scheme_pure_cn_passes() {
        let r = test_rule();
        let (cn_res, arabic_res) = heading_default_regexes();
        let c = HeadingNumberingChecker {
            allowed_schemes: vec!["cn_mixed".into()],
            require_consistent: true,
            check_level_match: true,
            cn_res,
            arabic_res,
        };
        let mut doc = empty_doc();
        doc.paragraphs = vec![
            abs_para("一、引言", ParagraphRole::Heading { level: 1 }),
            abs_para("二、方法", ParagraphRole::Heading { level: 1 }),
            abs_para("（一）背景", ParagraphRole::Heading { level: 2 }),
            abs_para("二十三、附录", ParagraphRole::Heading { level: 1 }),
        ];
        set_idx(&mut doc);
        assert!(c.check_document(&ctx(&doc, &r)).is_empty());
    }

    #[test]
    fn citation_order_ascending_passes() {
        let r = test_rule();
        let c = CitationOrderChecker { require_ascending: true, allow_repeated: true };
        let mut doc = empty_doc();
        doc.citations = [1u32, 2, 3]
            .iter()
            .enumerate()
            .map(|(i, n)| CitationOccurrence { paragraph_index: i, run_index: None, raw: format!("[{n}]"), numbers: vec![*n], page_hint: None })
            .collect();
        doc.reference_entries = (1..=3)
            .map(|n| ReferenceEntry { paragraph_index: n as usize, number: n, raw: format!("[{n}] 文献"), type_tag: Some("J".into()), has_year: true, has_pages: false, has_journal_or_publisher: true })
            .collect();
        assert!(c.check_document(&ctx(&doc, &r)).is_empty());
    }

    #[test]
    fn citation_order_descending_and_dangling_warn() {
        let r = test_rule();
        let c = CitationOrderChecker { require_ascending: true, allow_repeated: false };
        let mut doc = empty_doc();
        doc.citations = [3u32, 2, 1, 3, 2, 9]
            .iter()
            .enumerate()
            .map(|(i, n)| CitationOccurrence { paragraph_index: i, run_index: None, raw: format!("[{n}]"), numbers: vec![*n], page_hint: None })
            .collect();
        doc.reference_entries = (1..=3)
            .map(|n| ReferenceEntry { paragraph_index: n as usize, number: n, raw: format!("[{n}] 文献"), type_tag: Some("J".into()), has_year: true, has_pages: false, has_journal_or_publisher: true })
            .collect();
        let vs = c.check_document(&ctx(&doc, &r));
        assert!(vs.iter().any(|v| v.message.contains("回跳")));
        assert!(vs.iter().any(|v| v.message.contains("[9]")), "引而未列应报告");
        assert!(vs.iter().all(|v| v.severity == Severity::Warning && v.requires_manual_review));
    }

    #[test]
    fn repeated_adjacent_citation_allowed() {
        let r = test_rule();
        let c = CitationOrderChecker { require_ascending: true, allow_repeated: true };
        let mut doc = empty_doc();
        doc.citations = [1u32, 1, 2, 3]
            .iter()
            .enumerate()
            .map(|(i, n)| CitationOccurrence { paragraph_index: i, run_index: None, raw: format!("[{n}]"), numbers: vec![*n], page_hint: None })
            .collect();
        doc.reference_entries = (1..=3)
            .map(|n| ReferenceEntry { paragraph_index: n as usize, number: n, raw: format!("[{n}] 文献"), type_tag: Some("J".into()), has_year: true, has_pages: false, has_journal_or_publisher: true })
            .collect();
        assert!(c.check_document(&ctx(&doc, &r)).is_empty(), "紧邻重复引用不应判为回跳");
    }

    #[test]
    fn cn_num_parses_expected() {
        assert_eq!(cn_num_to_u32("一"), Some(1));
        assert_eq!(cn_num_to_u32("十"), Some(10));
        assert_eq!(cn_num_to_u32("二十三"), Some(23));
        assert_eq!(cn_num_to_u32("百"), None);
    }
}
