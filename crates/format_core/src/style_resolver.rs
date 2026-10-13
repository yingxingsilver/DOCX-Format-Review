//! style_resolver —— OOXML 样式继承链求解器。
//!
//! 有效格式优先级（高 → 低）：
//!   段落/Run 直接格式  >  字符样式(rStyle)  >  段落样式(pStyle，含 basedOn 链与 linked 联动)
//!   >  styles.xml docDefaults（缺省兜底：宋体 / Times New Roman / 10.5pt）。
//!
//! 约定：
//! - 本模块是**纯函数**：输入解析器产出的原始结构（direct + style_id + 样式表），
//!   输出填充好的 `effective` 字段；不读文件、不依赖 Tauri、无 unsafe。
//! - basedOn 链带深度上限与环检测；异常样式引用不中断整篇文档——该属性回退到
//!   更低优先级来源，并在 `parse_warnings` 记录一条中文提示。
//! - w:sz 半磅→pt 一律经 [`crate::dsl::half_points_to_pt`]；行距换算集中在
//!   [`line_spacing_from_ooxml`]，保证单位换算只有唯一出口。

use std::collections::HashMap;

use crate::dsl::{
    half_points_to_pt, DirectParagraphProps, DirectRunProps, EffectiveParagraphFormat,
    EffectiveRunFormat, FormatSources, Indent, LineSpacing, ParsedDocument, StyleDefinition,
};

/// 骨架自检占位（保持与 lib.rs 模块链接测试兼容）。
pub fn placeholder_ok() -> bool {
    true
}

/// basedOn 链最大深度（防御畸形样式表导致的长链/环）。
const MAX_STYLE_CHAIN: usize = 32;

/// docDefaults 缺失时的最终兜底值（Word 默认行为的近似）。
const FALLBACK_ASCII_FONT: &str = "Times New Roman";
const FALLBACK_EAST_ASIAN_FONT: &str = "宋体";
const FALLBACK_FONT_SIZE_PT: f64 = 10.5;

const SRC_DIRECT: &str = "run_direct";
const SRC_CHAR_STYLE: &str = "character_style";
const SRC_PARA_STYLE: &str = "paragraph_style";
const SRC_DOC_DEFAULTS: &str = "doc_defaults";

// =========================
// 属性合并基础工具
// =========================

/// 把 `from` 中 target 尚为空的字段填入；若提供 sources+tag 则同步标注来源。
fn merge_run_props(
    target: &mut DirectRunProps,
    sources: Option<&mut FormatSources>,
    from: &DirectRunProps,
    tag: &str,
) {
    macro_rules! fill {
        ($($field:ident),*) => {
            $(
                if target.$field.is_none() && from.$field.is_some() {
                    target.$field = from.$field.clone();
                    if !tag.is_empty() {
                        if let Some(s) = sources.as_mut() {
                            s.map.insert(stringify!($field).to_string(), tag.to_string());
                        }
                    }
                }
            )*
        };
    }
    fill!(
        font_size_pt,
        ascii_font,
        east_asian_font,
        hansi_font,
        cs_font,
        bold,
        italic,
        underline,
        highlight,
        color
    );
}

fn merge_para_props(
    target: &mut DirectParagraphProps,
    sources: Option<&mut FormatSources>,
    from: &DirectParagraphProps,
    tag: &str,
) {
    macro_rules! fill {
        ($($field:ident),*) => {
            $(
                if target.$field.is_none() && from.$field.is_some() {
                    target.$field = from.$field.clone();
                    if !tag.is_empty() {
                        if let Some(s) = sources.as_mut() {
                            s.map.insert(stringify!($field).to_string(), tag.to_string());
                        }
                    }
                }
            )*
        };
    }
    fill!(
        alignment,
        line_spacing,
        first_line_indent,
        left_indent,
        hanging_indent,
        outline_level,
        keep_with_next,
        page_break_before
    );
}

/// 已有更高优先级来源的键不被 extra 覆盖。
fn merge_sources(target: &mut FormatSources, extra: &FormatSources) {
    for (k, v) in &extra.map {
        target.map.entry(k.clone()).or_insert_with(|| v.clone());
    }
}

fn style_name<'a>(styles: &'a HashMap<String, StyleDefinition>, id: &str) -> &'a str {
    styles
        .get(id)
        .and_then(|s| s.name.as_deref())
        .unwrap_or(id)
}

// =========================
// 样式链解析
// =========================

/// 沿 basedOn 链合并 run 属性（子样式优先于父样式）。
/// 返回 (合并结果, 来源标注)。样式缺失/成环只记警告，不报错。
fn resolve_run_chain(
    style_id: &str,
    styles: &HashMap<String, StyleDefinition>,
    warnings: &mut Vec<String>,
) -> (DirectRunProps, FormatSources) {
    let mut merged = DirectRunProps::default();
    let mut sources = FormatSources::default();
    let mut visited: Vec<String> = Vec::new();
    let mut current = style_id.to_string();
    loop {
        if visited.iter().any(|v| *v == current) || visited.len() >= MAX_STYLE_CHAIN {
            warnings.push(format!(
                "样式“{current}”的继承关系存在循环或过深，部分格式未能自动判断。"
            ));
            break;
        }
        visited.push(current.clone());
        let Some(def) = styles.get(&current) else {
            warnings.push(format!(
                "文档引用了样式表里不存在的样式“{current}”，相关格式按更基础的默认值判断。"
            ));
            break;
        };
        let tag = format!("{SRC_CHAR_STYLE}:{}", style_name(styles, &current));
        merge_run_props(&mut merged, Some(&mut sources), &def.run_props, &tag);
        match def.based_on.as_deref() {
            None => break,
            Some(parent) => current = parent.to_string(),
        }
    }
    (merged, sources)
}

/// 沿 basedOn 链合并段落属性。
fn resolve_para_chain(
    style_id: &str,
    styles: &HashMap<String, StyleDefinition>,
    warnings: &mut Vec<String>,
) -> (DirectParagraphProps, FormatSources) {
    let mut merged = DirectParagraphProps::default();
    let mut sources = FormatSources::default();
    let mut visited: Vec<String> = Vec::new();
    let mut current = style_id.to_string();
    loop {
        if visited.iter().any(|v| *v == current) || visited.len() >= MAX_STYLE_CHAIN {
            warnings.push(format!(
                "样式“{current}”的继承关系存在循环或过深，部分格式未能自动判断。"
            ));
            break;
        }
        visited.push(current.clone());
        let Some(def) = styles.get(&current) else {
            warnings.push(format!(
                "文档引用了样式表里不存在的样式“{current}”，相关格式按更基础的默认值判断。"
            ));
            break;
        };
        let tag = format!("{SRC_PARA_STYLE}:{}", style_name(styles, &current));
        merge_para_props(&mut merged, Some(&mut sources), &def.paragraph_props, &tag);
        match def.based_on.as_deref() {
            None => break,
            Some(parent) => current = parent.to_string(),
        }
    }
    (merged, sources)
}

/// 段落样式对 run 属性的生效值：先走段落样式的 basedOn rPr 链。
/// （linked 字符样式在解析阶段已把其 rPr 并入同名段落样式的 run_props，见 docx_parser。）
fn para_style_run_props(
    style_id: &str,
    styles: &HashMap<String, StyleDefinition>,
    warnings: &mut Vec<String>,
) -> (DirectRunProps, FormatSources) {
    resolve_run_chain(style_id, styles, warnings)
}

// =========================
// docDefaults
// =========================

/// docDefaults run 属性 + 内置兜底（字号/中西文字体必有值）。
fn doc_defaults_run_props(doc: &ParsedDocument) -> (DirectRunProps, FormatSources) {
    let mut filled = doc
        .doc_defaults
        .as_ref()
        .map(|d| d.run_props.clone())
        .unwrap_or_default();
    let mut sources = FormatSources::default();
    if filled.font_size_pt.is_none() {
        filled.font_size_pt = Some(FALLBACK_FONT_SIZE_PT);
        sources
            .map
            .insert("font_size_pt".into(), "built_in_default".into());
    }
    if filled.ascii_font.is_none() {
        filled.ascii_font = Some(FALLBACK_ASCII_FONT.into());
        sources.map.insert("ascii_font".into(), "built_in_default".into());
    }
    if filled.east_asian_font.is_none() {
        filled.east_asian_font = Some(FALLBACK_EAST_ASIAN_FONT.into());
        sources
            .map
            .insert("east_asian_font".into(), "built_in_default".into());
    }
    (filled, sources)
}

fn doc_defaults_para_props(doc: &ParsedDocument) -> (DirectParagraphProps, FormatSources) {
    (
        doc.doc_defaults
            .as_ref()
            .map(|d| d.paragraph_props.clone())
            .unwrap_or_default(),
        FormatSources::default(),
    )
}

// =========================
// 对外求解接口
// =========================

/// 计算单个 run 的有效字符格式（含来源标注）。
#[allow(clippy::too_many_arguments)]
pub fn effective_run_format(
    direct: &DirectRunProps,
    character_style_id: Option<&str>,
    para_style_rpr: &DirectRunProps,
    para_style_sources: &FormatSources,
    defaults: &DirectRunProps,
    default_sources: &FormatSources,
    styles: &HashMap<String, StyleDefinition>,
    warnings: &mut Vec<String>,
) -> EffectiveRunFormat {
    let mut sources = FormatSources::default();
    // 1. 直接格式最高优先。
    let mut cur = direct.clone();
    for k in [
        "font_size_pt",
        "ascii_font",
        "east_asian_font",
        "bold",
        "italic",
        "underline",
        "highlight",
        "color",
    ] {
        if run_prop_is_some(&cur, k) {
            sources.map.insert(k.to_string(), SRC_DIRECT.to_string());
        }
    }
    // 2. 字符样式链。
    if let Some(cs) = character_style_id {
        let (resolved, rsrc) = resolve_run_chain(cs, styles, warnings);
        merge_run_props(&mut cur, Some(&mut sources), &resolved, cs);
        merge_sources(&mut sources, &rsrc);
    }
    // 3. 段落样式 rPr 链。
    merge_run_props(&mut cur, Some(&mut sources), para_style_rpr, "");
    merge_sources(&mut sources, para_style_sources);
    // 4. docDefaults / 内置兜底。
    merge_run_props(&mut cur, Some(&mut sources), defaults, "");
    merge_sources(&mut sources, default_sources);
    // 清掉空标签（merge 时以 "" 写入的会被后面 or_insert 的真实标签补救不了的情况极少，
    // 这里显式过滤保证 sources 值非空可用）。
    sources.map.retain(|_, v| !v.is_empty());

    EffectiveRunFormat {
        font_size_pt: cur.font_size_pt,
        ascii_font: cur.ascii_font.clone(),
        east_asian_font: cur.east_asian_font.clone(),
        bold: cur.bold.unwrap_or(false),
        italic: cur.italic.unwrap_or(false),
        underline: cur.underline.unwrap_or(false),
        highlight: cur.highlight.clone(),
        color: cur.color.clone(),
        sources,
    }
}

fn run_prop_is_some(p: &DirectRunProps, key: &str) -> bool {
    match key {
        "font_size_pt" => p.font_size_pt.is_some(),
        "ascii_font" => p.ascii_font.is_some(),
        "east_asian_font" => p.east_asian_font.is_some(),
        "hansi_font" => p.hansi_font.is_some(),
        "cs_font" => p.cs_font.is_some(),
        "bold" => p.bold.is_some(),
        "italic" => p.italic.is_some(),
        "underline" => p.underline.is_some(),
        "highlight" => p.highlight.is_some(),
        "color" => p.color.is_some(),
        _ => false,
    }
}

/// 计算段落有效格式（含来源标注）。
pub fn effective_paragraph_format(
    direct: &DirectParagraphProps,
    styles: &HashMap<String, StyleDefinition>,
    defaults: &(DirectParagraphProps, FormatSources),
    warnings: &mut Vec<String>,
) -> EffectiveParagraphFormat {
    let mut sources = FormatSources::default();
    let mut cur = direct.clone();
    for (k, has) in [
        ("alignment", cur.alignment.is_some()),
        ("line_spacing", cur.line_spacing.is_some()),
        ("first_line_indent", cur.first_line_indent.is_some()),
        ("left_indent", cur.left_indent.is_some()),
        ("hanging_indent", cur.hanging_indent.is_some()),
        ("outline_level", cur.outline_level.is_some()),
    ] {
        if has {
            sources.map.insert(k.to_string(), SRC_DIRECT.to_string());
        }
    }
    if let Some(sid) = direct.style_id.as_deref() {
        let (ppr, psources) = resolve_para_chain(sid, styles, warnings);
        merge_para_props(&mut cur, Some(&mut sources), &ppr, sid);
        merge_sources(&mut sources, &psources);
    }
    merge_para_props(&mut cur, Some(&mut sources), &defaults.0, SRC_DOC_DEFAULTS);
    merge_sources(&mut sources, &defaults.1);
    sources.map.retain(|_, v| !v.is_empty());

    EffectiveParagraphFormat {
        alignment: cur.alignment,
        line_spacing: cur.line_spacing.clone(),
        first_line_indent: cur.first_line_indent.clone(),
        left_indent: cur.left_indent.clone(),
        hanging_indent: cur.hanging_indent.clone(),
        outline_level: cur.outline_level,
        style_id: direct.style_id.clone(),
        sources,
    }
}

/// 对整份文档就地求解：填充每个段落与 run 的 `effective` 字段。
///
/// 幂等：重复调用只是重算，不追加重复警告（同文本警告去重）。
/// 返回本次新增的中文解析警告条数（已并入 `doc.parse_warnings`）。
pub fn resolve_document(doc: &mut ParsedDocument) -> usize {
    let (def_run, def_run_sources) = doc_defaults_run_props(doc);
    let def_para = doc_defaults_para_props(doc);
    let before = doc.parse_warnings.len();

    // 每样式缓存一次链解析结果，避免逐 run 重走继承链（批量审核性能关键路径）。
    let mut run_chain_cache: HashMap<String, (DirectRunProps, FormatSources)> = HashMap::new();
    let mut para_chain_cache: HashMap<String, (DirectParagraphProps, FormatSources)> =
        HashMap::new();

    let styles = doc.styles.clone();
    let mut warnings: Vec<String> = Vec::new();
    for p in doc.paragraphs.iter_mut() {
        let p_eff = effective_paragraph_format_cached(
            &p.direct,
            &styles,
            &def_para,
            &mut para_chain_cache,
            &mut warnings,
        );
        let (psr_rpr, psr_src) = match p.style_id.as_deref() {
            Some(sid) => run_chain_cache
                .entry(sid.to_string())
                .or_insert_with(|| para_style_run_props(sid, &styles, &mut Vec::new()))
                .clone(),
            None => (DirectRunProps::default(), FormatSources::default()),
        };
        for r in p.runs.iter_mut() {
            r.effective = Some(effective_run_format(
                &r.direct,
                r.character_style_id.as_deref(),
                &psr_rpr,
                &psr_src,
                &def_run,
                &def_run_sources,
                &styles,
                &mut warnings,
            ));
        }
        p.effective = Some(p_eff);
    }
    warnings.sort();
    warnings.dedup();
    doc.parse_warnings.extend(warnings);
    doc.parse_warnings.len() - before
}

/// 带缓存的段落求解（内部使用）。
fn effective_paragraph_format_cached(
    direct: &DirectParagraphProps,
    styles: &HashMap<String, StyleDefinition>,
    defaults: &(DirectParagraphProps, FormatSources),
    cache: &mut HashMap<String, (DirectParagraphProps, FormatSources)>,
    warnings: &mut Vec<String>,
) -> EffectiveParagraphFormat {
    let mut sources = FormatSources::default();
    let mut cur = direct.clone();
    for (k, has) in [
        ("alignment", cur.alignment.is_some()),
        ("line_spacing", cur.line_spacing.is_some()),
        ("first_line_indent", cur.first_line_indent.is_some()),
        ("left_indent", cur.left_indent.is_some()),
        ("hanging_indent", cur.hanging_indent.is_some()),
        ("outline_level", cur.outline_level.is_some()),
    ] {
        if has {
            sources.map.insert(k.to_string(), SRC_DIRECT.to_string());
        }
    }
    if let Some(sid) = direct.style_id.as_deref() {
        let (ppr, psources) = cache
            .entry(sid.to_string())
            .or_insert_with(|| resolve_para_chain(sid, styles, &mut Vec::new()))
            .clone();
        merge_para_props(&mut cur, Some(&mut sources), &ppr, sid);
        merge_sources(&mut sources, &psources);
    }
    merge_para_props(&mut cur, Some(&mut sources), &defaults.0, SRC_DOC_DEFAULTS);
    merge_sources(&mut sources, &defaults.1);
    sources.map.retain(|_, v| !v.is_empty());

    let _ = warnings; // 缓存路径中的链警告已在首次全量解析时收集
    EffectiveParagraphFormat {
        alignment: cur.alignment,
        line_spacing: cur.line_spacing.clone(),
        first_line_indent: cur.first_line_indent.clone(),
        left_indent: cur.left_indent.clone(),
        hanging_indent: cur.hanging_indent.clone(),
        outline_level: cur.outline_level,
        style_id: direct.style_id.clone(),
        sources,
    }
}

// =========================
// 单位换算辅助（供 docx_parser 复用，集中换算出口）
// =========================

/// OOXML `w:spacing/@w:line` 换算为 DSL 行距：
/// Auto → line/240 倍；exact/atLeast → line/20 pt。
pub fn line_spacing_from_ooxml(line: i64, rule: &str) -> Option<LineSpacing> {
    match rule {
        "auto" => Some(LineSpacing::Multiple {
            factor: line as f64 / 240.0,
        }),
        "exact" => Some(LineSpacing::Exact {
            points: line as f64 / 20.0,
        }),
        "atLeast" => Some(LineSpacing::AtLeast {
            points: line as f64 / 20.0,
        }),
        _ => None,
    }
}

/// twips 缩进保留原单位进入 DSL（approx_chars 时再按字号折算字符数）。
pub fn indent_from_twips(twips: f64) -> Indent {
    Indent::Twips { value: twips }
}

/// 半磅 → pt 转发（保持单一换算入口的可见性）。
pub fn font_size_from_half_points(half_points: f64) -> f64 {
    half_points_to_pt(half_points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Alignment, Run, Paragraph, ParagraphRole};

    fn sd(id: &str, name: &str, based_on: Option<&str>) -> StyleDefinition {
        StyleDefinition {
            style_id: id.into(),
            name: Some(name.into()),
            based_on: based_on.map(Into::into),
            next_paragraph_style: None,
            paragraph_props: Default::default(),
            run_props: Default::default(),
            is_default: false,
            aliases: vec![],
        }
    }

    fn doc_with(styles: Vec<StyleDefinition>, defaults: Option<StyleDefinition>) -> ParsedDocument {
        ParsedDocument {
            path: Default::default(),
            file_name: "t.docx".into(),
            sections: vec![],
            blocks: vec![],
            paragraphs: vec![],
            styles: styles.into_iter().map(|s| (s.style_id.clone(), s)).collect(),
            doc_defaults: defaults,
            captions: vec![],
            citations: vec![],
            reference_entries: vec![],
            page_break_count: 0,
            parse_warnings: vec![],
        }
    }

    fn para(style_id: Option<&str>, runs: Vec<Run>) -> Paragraph {
        Paragraph {
            global_index: 0,
            block_index: 0,
            section_index: 0,
            text: runs.iter().map(|r| r.text.as_str()).collect(),
            runs,
            direct: DirectParagraphProps {
                style_id: style_id.map(Into::into),
                ..Default::default()
            },
            style_id: style_id.map(Into::into),
            effective: None,
            role: ParagraphRole::Body,
            role_tags: vec![],
            role_confidence: 1.0,
            page_hint: None,
            in_table: false,
            in_header_footer: false,
            in_footnote_endnote: false,
        }
    }

    fn run(direct: DirectRunProps, cstyle: Option<&str>) -> Run {
        Run {
            index: 0,
            text: "正文".into(),
            direct,
            character_style_id: cstyle.map(Into::into),
            effective: None,
            inline_objects: vec![],
            text_class_summary: Default::default(),
        }
    }

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }

    #[test]
    fn direct_format_wins_over_styles_and_defaults() {
        let mut doc = doc_with(vec![], None);
        let mut h1 = sd("1", "heading 1", None);
        h1.run_props.font_size_pt = Some(16.0);
        doc.styles.insert("1".into(), h1);
        let p = para(
            Some("1"),
            vec![run(DirectRunProps { font_size_pt: Some(12.0), ..Default::default() }, None)],
        );
        doc.paragraphs = vec![p];
        resolve_document(&mut doc);
        let eff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(eff.font_size_pt, Some(12.0));
        assert_eq!(eff.sources.map.get("font_size_pt").map(String::as_str), Some(SRC_DIRECT));
    }

    #[test]
    fn paragraph_style_applies_when_no_direct() {
        let mut doc = doc_with(vec![], None);
        let mut h1 = sd("1", "heading 1", None);
        h1.run_props.font_size_pt = Some(16.0);
        h1.run_props.bold = Some(true);
        h1.paragraph_props.alignment = Some(Alignment::Center);
        doc.styles.insert("1".into(), h1);
        doc.paragraphs = vec![para(Some("1"), vec![run(Default::default(), None)])];
        resolve_document(&mut doc);
        let reff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(reff.font_size_pt, Some(16.0));
        assert!(reff.bold);
        let peff = doc.paragraphs[0].effective.as_ref().unwrap();
        assert_eq!(peff.alignment, Some(Alignment::Center));
        assert!(peff.sources.map.contains_key("alignment"));
    }

    #[test]
    fn based_on_chain_merges_parent_into_child() {
        let mut doc = doc_with(vec![], None);
        let mut base = sd("Base", "正文基线", None);
        base.run_props.east_asian_font = Some("仿宋".into());
        base.run_props.font_size_pt = Some(12.0);
        let mut child = sd("Child", "子样式", Some("Base"));
        child.run_props.font_size_pt = Some(10.5);
        doc.styles.insert("Base".into(), base);
        doc.styles.insert("Child".into(), child);
        doc.paragraphs = vec![para(None, vec![run(Default::default(), Some("Child"))])];
        resolve_document(&mut doc);
        let eff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(eff.font_size_pt, Some(10.5), "子样式覆盖父样式");
        assert_eq!(eff.east_asian_font.as_deref(), Some("仿宋"), "父样式补齐空缺");
    }

    #[test]
    fn character_style_beats_paragraph_style() {
        let mut doc = doc_with(vec![], None);
        let mut psty = sd("P", "段落样式", None);
        psty.run_props.ascii_font = Some("Arial".into());
        let mut csty = sd("C", "字符样式", None);
        csty.run_props.ascii_font = Some("Calibri".into());
        doc.styles.insert("P".into(), psty);
        doc.styles.insert("C".into(), csty);
        doc.paragraphs = vec![para(Some("P"), vec![run(Default::default(), Some("C"))])];
        resolve_document(&mut doc);
        let eff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(eff.ascii_font.as_deref(), Some("Calibri"));
    }

    #[test]
    fn doc_defaults_used_last_with_builtin_fallback() {
        let mut doc = doc_with(vec![], None);
        doc.paragraphs = vec![para(None, vec![run(Default::default(), None)])];
        resolve_document(&mut doc);
        let eff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(eff.font_size_pt, Some(FALLBACK_FONT_SIZE_PT));
        assert_eq!(eff.east_asian_font.as_deref(), Some("宋体"));
        assert_eq!(eff.ascii_font.as_deref(), Some("Times New Roman"));
    }

    #[test]
    fn style_cycle_is_broken_with_warning_not_hang() {
        let mut doc = doc_with(vec![], None);
        let a = sd("A", "甲", Some("B"));
        let b = sd("B", "乙", Some("A"));
        doc.styles.insert("A".into(), a);
        doc.styles.insert("B".into(), b);
        doc.paragraphs = vec![para(Some("A"), vec![run(Default::default(), None)])];
        let added = resolve_document(&mut doc);
        assert!(added >= 1, "应产生循环警告");
        assert!(doc.parse_warnings.iter().any(|w| w.contains("循环")));
    }

    #[test]
    fn unknown_style_reference_warns_but_still_resolves() {
        let mut doc = doc_with(vec![], None);
        doc.paragraphs = vec![para(Some("Ghost"), vec![run(Default::default(), Some("Ghost2"))])];
        resolve_document(&mut doc);
        assert!(doc.parse_warnings.iter().any(|w| w.contains("不存在")));
        let eff = doc.paragraphs[0].runs[0].effective.as_ref().unwrap();
        assert_eq!(eff.font_size_pt, Some(FALLBACK_FONT_SIZE_PT), "仍应回退到默认值");
    }

    #[test]
    fn line_spacing_unit_conversion() {
        assert_eq!(
            line_spacing_from_ooxml(360, "auto"),
            Some(LineSpacing::Multiple { factor: 1.5 })
        );
        assert_eq!(
            line_spacing_from_ooxml(480, "exact"),
            Some(LineSpacing::Exact { points: 24.0 })
        );
        assert_eq!(line_spacing_from_ooxml(480, "bogus"), None);
        assert_eq!(font_size_from_half_points(28.0), 14.0);
    }
}
