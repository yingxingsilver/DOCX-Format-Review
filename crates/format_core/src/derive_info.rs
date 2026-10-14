//! derive_info —— 解析阶段的派生信息提取（题注 / 文内引用 / 参考文献条目 / 角色粗分类）。
//!
//! 设计约定：
//! - 输入是**未经样式求解的原始段落**（文本 + 样式 ID + 表格标记），输出直接填充到
//!   [`ParsedDocument`] 的 `captions` / `citations` / `reference_entries` / `role_tags` 字段，
//!   并对高置信命中的段落回填 `role`（题注、参考文献条目）——规则引擎只消费这些事实，
//!   不再各自重复扫全文（见 checkers/caption.rs 的判定原则说明）。
//! - 正则一律进程级 `OnceLock` 编译一次，跨线程只读共享（批量审核走 rayon）。
//! - 本模块绝不做“格式对不对”的判断，只做“是什么”的识别；识别不了就留空/Unknown，
//!   由检查器按低置信度降级为人工复核，而不是在这里猜。

use std::sync::OnceLock;

use regex::Regex;

use crate::dsl::{
    CaptionInfo, CaptionKind, CaptionPosition, CitationOccurrence, Paragraph, ParagraphRole,
    ParsedDocument, ReferenceEntry, RoleTag,
};

/// 题注前缀：图 / 表 / 附图（含英文 Figure/Fig./Table 缩写）。
fn caption_prefix_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(附?[图表]|Figure|Fig\.?|Table)\s*([0-9]+(?:[-.．][0-9]+)*)").expect("静态正则必然合法")
    })
}

/// 文内引用：[1] / [1,2] / [1-3] / [1，2] / [1][2]（连续组各算一次出现）。
fn citation_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[[0-9]+(?:\s*[-–,，]\s*[0-9]+)*\]").expect("静态正则必然合法"))
}

/// 编号序列提取（“1-2” → [1,2]，“1,3” → [1,3]）。
fn numbers_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[0-9]+").expect("静态正则必然合法"))
}

/// 参考文献条目起始：`[1]` / `[01]`。
fn ref_entry_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\[(0?[1-9][0-9]*)\]").expect("静态正则必然合法"))
}

/// 年份（19xx / 20xx）。
fn year_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(19|20)[0-9]{2}").expect("静态正则必然合法"))
}

/// GB/T 7714 文献类型标识：`[M]` `[J]` `[C]` `[D]` 等（单大写字母，允许前后空格）。
fn type_tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[\s*([MJCDRSP])\w?\s*\]").expect("静态正则必然合法"))
}

/// 页码区间：`123-129` / `123~129` / `123—129`。
fn pages_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[0-9]+\s*[-~—至]\s*[0-9]+").expect("静态正则必然合法"))
}

/// “刊名/出版社”启发式特征词（仅用于 has_journal_or_publisher 布尔位）。
fn publisher_hints(text: &str) -> bool {
    const CN: [&str; 8] = ["学报", "出版社", "书局", "周刊", "研究", "评论", "杂志", "导刊"];
    const EN: [&str; 5] = ["Journal", "Press", "Proceedings", "Review", "Transactions"];
    CN.iter().any(|k| text.contains(k)) || EN.iter().any(|k| text.contains(k))
}

/// 把 “1-2” / “1.2” / “3” 之类的编号串拆成数字序列。
fn split_number_parts(s: &str) -> Vec<u32> {
    s.split(['-', '.', '．'])
        .filter_map(|p| p.trim().parse::<u32>().ok())
        .collect()
}

/// 从文内引用原文提取编号列表（“[1-3]” → [1,3]，范围展开交由检查器解释）。
pub fn citation_numbers(raw: &str) -> Vec<u32> {
    numbers_re()
        .find_iter(raw)
        .filter_map(|m| m.as_str().parse::<u32>().ok())
        .collect()
}

/// 识别一条题注；不是题注返回 None。
///
/// 置信度策略：
/// - 样式名含“题注/Caption”→ 0.9；
/// - 文本以 图X/表X 开头且较短 → 0.75；
/// - 其余（长句恰好以“分析表明…”开头等）→ 不识别或 ≤0.6 并要求人工复核。
pub fn detect_caption(p: &Paragraph) -> Option<CaptionInfo> {
    let text = p.text.trim();
    if text.is_empty() || text.chars().count() > 80 {
        return None;
    }
    let m = caption_prefix_re().find(text)?;
    let matched = m.as_str();
    let label_end = matched.find([' ', '　']).unwrap_or(matched.len());
    let (label, num_str) = matched.split_at(label_end);
    let kind = match label {
        "图" | "附图" | "Figure" | "Fig" | "Fig." => CaptionKind::Figure,
        "表" | "附表" | "Table" => CaptionKind::Table,
        _ => return None,
    };
    // “图2-1 系统架构”与“如图所示……”区分：编号后必须跟分隔符或结束。
    let rest = text[m.end()..].trim_start();
    let looks_like_caption = !rest.is_empty() && !starts_with_verb_like(rest);
    let style_says_caption = p
        .style_id
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("Caption") || s.contains("题注"));
    let confidence = if style_says_caption {
        0.9
    } else if looks_like_caption {
        0.75
    } else {
        0.55
    };
    if confidence < 0.6 {
        return None;
    }
    Some(CaptionInfo {
        paragraph_index: p.global_index,
        kind,
        raw_label: label.to_string(),
        number_parts: split_number_parts(num_str.trim()),
        title_text: if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        },
        nearby_object_block_index: None,
        position: CaptionPosition::Unknown,
        confidence,
    })
}

/// “如图/如表/图中/表中…”等叙述性开头——排除误报（“分析”“图表显示趋势明显”类长句已由长度上限兜底）。
fn starts_with_verb_like(rest: &str) -> bool {
    rest.starts_with("所示")
        || rest.starts_with("中")
        || rest.starts_with("显示")
        || rest.starts_with("的数据")
}

/// 提取一个段落内的全部文内引用出现。
pub fn extract_citations(p: &Paragraph) -> Vec<CitationOccurrence> {
    let mut out = Vec::new();
    for m in citation_re().find_iter(&p.text) {
        let raw = m.as_str().to_string();
        let numbers = citation_numbers(&raw);
        if numbers.is_empty() {
            continue;
        }
        out.push(CitationOccurrence {
            paragraph_index: p.global_index,
            run_index: None,
            raw,
            numbers,
            page_hint: p.page_hint,
        });
    }
    out
}

/// 识别文末参考文献条目（`[1] 作者. 题名[J]. 刊名, 年份(期): 页.`）。
///
/// 仅在进入“参考文献”区域后才尝试识别，避免把正文里的 `[1]` 误当条目。
pub fn detect_reference_entry(p: &Paragraph, in_refs_section: bool) -> Option<ReferenceEntry> {
    if !in_refs_section {
        return None;
    }
    let text = p.text.trim();
    let m = ref_entry_re().find(text)?;
    let number: u32 = m.as_str()[1..m.len() - 1].trim_start_matches('0').max("0").parse().ok()?;
    let type_tag = type_tag_re()
        .captures(text)
        .and_then(|c| c.get(1))
        .map(|g| g.as_str().to_string());
    Some(ReferenceEntry {
        paragraph_index: p.global_index,
        number,
        raw: text.to_string(),
        type_tag,
        has_year: year_re().is_match(text),
        has_pages: pages_re().is_match(text),
        has_journal_or_publisher: publisher_hints(text),
    })
}

/// 标题样式名 → 层级（“标题 1”/“heading 1”/“H1”）。
fn heading_level_from_style(style_id: &str) -> Option<u8> {
    let s = style_id.trim();
    if let Some(rest) = s.strip_prefix("heading ") {
        return rest.trim().parse::<u8>().ok().filter(|l| (1..=9).contains(l));
    }
    if s.starts_with("标题") {
        return s[2..].trim().parse::<u8>().ok().filter(|l| (1..=9).contains(l));
    }
    if s.len() == 2 && s.starts_with('H') {
        return s[1..].parse::<u8>().ok().filter(|l| (1..=9).contains(l));
    }
    None
}

/// 对尚未定角色的段落做保守粗分类（样式驱动优先，文本特征其次）。
///
/// 已有具体角色（如题注）的段落不覆盖；结果多为 Body/Other/Heading，
/// 无法确定的保持 Unknown 并打 `manual_review_recommended` 标签。
pub fn classify_roles(paragraphs: &mut [Paragraph]) {
    let mut refs_started = false;
    for p in paragraphs.iter_mut() {
        let text = p.text.trim();
        if text.is_empty() {
            continue;
        }
        // 参考文献区边界：遇到“致谢/附录”即退出。
        if text.starts_with("参考文献") {
            refs_started = true;
            p.role_tags.push(RoleTag::StartsReferences);
            continue;
        }
        if text.starts_with("致谢") || text.starts_with("附录") {
            refs_started = false;
            continue;
        }
        if !matches!(p.role, ParagraphRole::Unknown) {
            continue;
        }
        if let Some(sid) = p.style_id.as_deref() {
            if let Some(level) = heading_level_from_style(sid) {
                p.role = ParagraphRole::Heading { level };
                p.role_confidence = 0.9;
                continue;
            }
        }
        if refs_started {
            p.role = ParagraphRole::ReferenceEntry;
            p.role_confidence = 0.8;
            continue;
        }
        // 无样式依据时保守处理：短行可能是标题，但不猜测——标 Other 供规则自行匹配文本。
        p.role = ParagraphRole::Other;
        p.role_confidence = 0.4;
        p.role_tags.push(RoleTag::ManualReviewRecommended);
    }
}

/// 在原始段落数组上运行全部派生提取，就地填充文档字段。
///
/// 顺序要求：先角色粗分类（确定参考文献区），再逐段提取题注/引用/条目；
/// 最后把高置信题注回填为 `figure_caption` / `table_caption` 角色（caption 检查器直接消费）。
pub fn populate_derived(doc: &mut ParsedDocument) {
    classify_roles(&mut doc.paragraphs);

    // 先只读扫描收集事实，再统一写回——避免边迭代边改借用的下标错位问题。
    let mut captions = Vec::new();
    let mut citations = Vec::new();
    let mut entries = Vec::new();
    let mut drawing_paras = Vec::new();
    let mut in_refs = false;
    for p in doc.paragraphs.iter() {
        let text = p.text.trim();
        if text.starts_with("参考文献") {
            in_refs = true;
        }
        if let Some(c) = detect_caption(p) {
            captions.push(c);
        }
        citations.extend(extract_citations(p));
        if let Some(e) = detect_reference_entry(p, in_refs) {
            entries.push(e);
        }
        if !p.in_table && p.runs.iter().any(|r| !r.inline_objects.is_empty()) {
            // 含内嵌对象的段落：给邻近规则一个事实标签（global_index 即下标，由解析器保证）。
            drawing_paras.push(p.global_index);
        }
    }
    for idx in drawing_paras {
        doc.paragraphs[idx].role_tags.push(RoleTag::HasDrawing);
    }
    for c in &captions {
        if c.confidence >= 0.6 {
            let role = match c.kind {
                CaptionKind::Figure => ParagraphRole::FigureCaption,
                CaptionKind::Table => ParagraphRole::TableCaption,
            };
            let p = &mut doc.paragraphs[c.paragraph_index];
            if matches!(p.role, ParagraphRole::Unknown | ParagraphRole::Other) {
                p.role = role;
                p.role_confidence = c.confidence;
            }
        }
    }
    doc.captions = captions;
    doc.citations = citations;
    doc.reference_entries = entries;

    // 解析阶段事实回填：InTable/InHeader/ContainsCitation/FirstNonEmpty 标签、
    // 题注邻近对象块与相对位置（由 docx_parser::annotate_derived_facts 提供）。
    crate::docx_parser::annotate_derived_facts(doc);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::Run;

    fn para(idx: usize, text: &str, style: Option<&str>) -> Paragraph {
        Paragraph {
            global_index: idx,
            block_index: idx,
            section_index: 0,
            text: text.to_string(),
            runs: vec![Run {
                index: 0,
                text: text.to_string(),
                direct: Default::default(),
                character_style_id: None,
                effective: None,
                inline_objects: vec![],
                text_class_summary: Default::default(),
            }],
            direct: Default::default(),
            style_id: style.map(|s| s.to_string()),
            effective: None,
            role: ParagraphRole::Unknown,
            role_tags: vec![],
            role_confidence: 0.0,
            page_hint: None,
            in_table: false,
            in_header_footer: false,
            in_footnote_endnote: false,
        }
    }

    #[test]
    fn detects_figure_caption_with_number_and_title() {
        let c = detect_caption(&para(3, "图2-1 系统总体架构图", None)).expect("应识别为题注");
        assert_eq!(c.kind, CaptionKind::Figure);
        assert_eq!(c.number_parts, vec![2, 1]);
        assert_eq!(c.title_text.as_deref(), Some("系统总体架构图"));
        assert!(c.confidence >= 0.7);
    }

    #[test]
    fn caption_style_boosts_confidence() {
        let a = detect_caption(&para(0, "表3 实验数据", Some("Caption"))).unwrap();
        let b = detect_caption(&para(0, "表3 实验数据", None)).unwrap();
        assert!(a.confidence > b.confidence);
        assert_eq!(a.kind, CaptionKind::Table);
    }

    #[test]
    fn narrative_like_text_is_not_caption() {
        // “如图所示……”叙述句不应被当成题注。
        assert!(detect_caption(&para(0, "图所示为本研究的整体流程说明文字", None)).is_none()
            || detect_caption(&para(0, "图所示为本研究的整体流程说明文字", None))
                .unwrap()
                .confidence
                < 0.6);
    }

    #[test]
    fn extracts_bracket_citations_including_ranges() {
        let cs = extract_citations(&para(
            7,
            "该方法在多个领域得到验证[1]，后续研究[2-4]进一步扩展了适用范围[5，6]。",
        ));
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[0].numbers, vec![1]);
        assert_eq!(cs[1].numbers, vec![2, 4]);
        assert_eq!(cs[2].numbers, vec![5, 6]);
        assert_eq!(cs[1].paragraph_index, 7);
    }

    #[test]
    fn reference_entry_requires_section_and_has_flags() {
        let text = "[12] 张三. 教育公平量化研究[J]. 教育研究, 2021(3): 12-19.";
        // 未进入参考文献区 → 不识别。
        assert!(detect_reference_entry(&para(20, text), false).is_none());
        let e = detect_reference_entry(&para(20, text), true).expect("应识别为条目");
        assert_eq!(e.number, 12);
        assert_eq!(e.type_tag.as_deref(), Some("J"));
        assert!(e.has_year && e.has_pages && e.has_journal_or_publisher);
    }

    #[test]
    fn heading_style_maps_to_level() {
        assert_eq!(heading_level_from_style("heading 1"), Some(1));
        assert_eq!(heading_level_from_style("标题 2"), Some(2));
        assert_eq!(heading_level_from_style("H3"), Some(3));
        assert_eq!(heading_level_from_style("Normal"), None);
    }

    #[test]
    fn populate_derived_fills_doc_and_marks_ref_region() {
        let mut doc = ParsedDocument {
            path: Default::default(),
            file_name: "t.docx".into(),
            sections: vec![],
            blocks: vec![],
            paragraphs: vec![
                para(0, "一级标题测试", Some("heading 1")),
                para(1, "本文方法已被验证[1]。", None),
                para(2, "图1 模型结构", None),
                para(3, "参考文献", None),
                para(4, "[1] 李四. 某研究[J]. 学报, 2020(1): 3-9.", None),
            ],
            styles: Default::default(),
            doc_defaults: None,
            captions: vec![],
            citations: vec![],
            reference_entries: vec![],
            page_break_count: 0,
            parse_warnings: vec![],
        };
        populate_derived(&mut doc);
        assert_eq!(doc.paragraphs[0].role, ParagraphRole::Heading { level: 1 });
        assert_eq!(doc.captions.len(), 1);
        assert_eq!(doc.captions[0].kind, CaptionKind::Figure);
        assert_eq!(doc.citations.len(), 1);
        assert_eq!(doc.reference_entries.len(), 1);
        assert_eq!(doc.reference_entries[0].number, 1);
        assert!(doc.paragraphs[4].role == ParagraphRole::ReferenceEntry);
        assert!(doc.paragraphs[3]
            .role_tags
            .contains(&RoleTag::StartsReferences));
    }
}
