//! docx_parser —— OOXML 解包与解析（zip + quick-xml 直读 XML，不依赖外部导出的 JSON）。
//!
//! 管线（[`parse_docx`]）：
//! 1. zip 打开 .docx → 读取 `word/document.xml`、`word/styles.xml`（styles 缺失时容错继续）；
//! 2. styles.xml → 样式表 [`StyleDefinition`] + docDefaults（含 basedOn / link 联动合并）；
//! 3. document.xml → 段落 / run / 表格块 / 节属性（页面尺寸、页边距），单位在此一次性换算
//!    （半磅→pt、twips→cm/mm，全部经 dsl 的换算函数）；
//! 4. [`crate::style_resolver::resolve_document`] 计算有效格式并填充 `effective`；
//! 5. [`crate::derive_info::populate_derived`] 提取题注、文内引用、参考文献条目等派生信息。
//!
//! 兼容性约定：若存在同名 `.parsed.json` sidecar（旧导出管线的产物），优先按原路径加载，
//! 保证既有工作流不被本阶段破坏；sidecar 缺失时走真实 XML 解析。
//!
//! 错误策略：损坏/加密/缺部件 → `CoreError::Zip/Xml/MissingPart`，上层 audit 转成
//! `FileStatus::ReadFailed` 中文友好文案；无法识别的节点只跳过或记 `parse_warnings`，绝不 panic。

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader;

use crate::dsl::{
    half_points_to_pt, twips_to_cm, twips_to_mm, Alignment, Block, DirectParagraphProps,
    DirectRunProps, InlineObject, InlineObjectKind, Margins, PageSize, Paragraph, ParagraphRole,
    ParsedDocument, Run, SectionProperties, StyleDefinition, TableBlock, TableCell, TableRow,
    TextClassSummary,
};
use crate::error::{CoreError, CoreResult};

/// 骨架自检占位（保持与 lib.rs 模块链接测试兼容）。
pub fn placeholder_ok() -> bool {
    true
}

/// 单个 XML 部件解压上限（防 zip 炸弹 OOM）。
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;

/// 解析一个 .docx 为完整 [`ParsedDocument`]（含有效格式与派生信息）。
pub fn parse_docx(path: &Path) -> CoreResult<ParsedDocument> {
    if !path.exists() {
        return Err(CoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "找不到这份文件".into(),
        });
    }
    let bytes = std::fs::read(path).map_err(|e| CoreError::Io {
        context: "打开论文文件".into(),
        source: e,
    })?;
    // 兼容旧导出管线：sidecar 存在即优先使用（见模块注释）。
    let json_path = sidecar_json(path);
    if json_path.exists() {
        return load_sidecar(path, &json_path);
    }
    if bytes.len() < 4 || bytes[0] != b'P' || bytes[1] != b'K' {
        return Err(CoreError::Zip {
            note: "这份文件无法识别，可能已损坏、是旧版格式或受密码保护。".into(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a zip container",
            )),
        });
    }

    let cursor = Cursor::new(&bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| CoreError::Zip {
            note: "这份文件无法识别，可能已损坏、是旧版格式或受密码保护。".into(),
            source: Box::new(e),
        })?;

    let document_xml = read_part(&mut archive, "word/document.xml")?.ok_or_else(|| {
        CoreError::MissingPart {
            part: "word/document.xml".into(),
        }
    })?;
    let styles_xml = read_part(&mut archive, "word/styles.xml").unwrap_or(None);

    let mut doc = new_document_shell(path);

    // —— styles.xml ——
    if let Some(xml) = &styles_xml {
        let (styles, defaults) = parse_styles_xml(xml)?;
        doc.styles = styles;
        doc.doc_defaults = defaults;
    }

    // —— document.xml ——
    let warnings = parse_document_xml(&document_xml, &mut doc)?;
    doc.parse_warnings.extend(warnings);

    // —— 样式求解 + 派生信息 ——
    crate::style_resolver::resolve_document(&mut doc);
    crate::derive_info::populate_derived(&mut doc);
    Ok(doc)
}

fn new_document_shell(path: &Path) -> ParsedDocument {
    ParsedDocument {
        path: path.to_path_buf(),
        file_name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        sections: Vec::new(),
        blocks: Vec::new(),
        paragraphs: Vec::new(),
        styles: HashMap::new(),
        doc_defaults: None,
        captions: Vec::new(),
        citations: Vec::new(),
        reference_entries: Vec::new(),
        page_break_count: 0,
        parse_warnings: Vec::new(),
    }
}

fn load_sidecar(path: &Path, json_path: &Path) -> CoreResult<ParsedDocument> {
    let text = std::fs::read_to_string(json_path).map_err(|e| CoreError::Io {
        context: "读取解析结果".into(),
        source: e,
    })?;
    let mut doc: ParsedDocument =
        serde_json::from_str(&text).map_err(|e| CoreError::SidecarInvalid {
            path: json_path.display().to_string(),
            source: Box::new(e),
        })?;
    doc.path = path.to_path_buf();
    doc.file_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    Ok(doc)
}

/// sidecar 路径：`foo.docx` → `foo.parsed.json`（与 audit 测试的命名约定一致）。
fn sidecar_json(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".parsed.json");
    PathBuf::from(p)
}

/// 读取 zip 内部件（UTF-8）；不存在返回 Ok(None)。带解压体积上限（zip 炸弹防护）。
fn read_part<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> CoreResult<Option<String>> {
    let file = match archive.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => {
            return Err(CoreError::Zip {
                note: format!("文档内部内容读取失败：{name}"),
                source: Box::new(e),
            })
        }
    };
    if file.size() > MAX_PART_BYTES {
        return Err(CoreError::Zip {
            note: "文档内容异常庞大，疑似损坏文件，已停止读取。".into(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "part too large",
            )),
        });
    }
    let mut buf = Vec::new();
    file.take(MAX_PART_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| CoreError::Zip {
            note: format!("文档内部内容读取失败：{name}"),
            source: Box::new(e),
        })?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| CoreError::Xml {
            part: name.into(),
            source: Box::new(e),
        })
}

// =========================
// 事件名工具（quick-xml 0.36：local() 返回 Cow<[u8]>）
// =========================

fn local_name(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.local_name().into_inner()).into_owned()
}

fn local_name_end(e: &BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.local_name().into_inner()).into_owned()
}

/// 读取命名空间无关属性（如 `w:val` 按 `val` 匹配）。
fn attr_val(e: &BytesStart<'_>, key: &str) -> Option<String> {
    for a in e.attributes() {
        if let Ok(a) = a {
            if a.key.local_name().into_inner() == key.as_bytes() {
                return Some(String::from_utf8_lossy(&a.value).into_owned());
            }
        }
    }
    None
}

fn attr_f64(e: &BytesStart<'_>, key: &str) -> Option<f64> {
    attr_val(e, key).and_then(|v| v.trim().parse().ok())
}

fn on_off(v: Option<String>) -> Option<bool> {
    match v.as_deref() {
        None => Some(true), // <w:b/> 无 val 即为开
        Some("1") | Some("true") | Some("on") => Some(true),
        Some("0") | Some("false") | Some("off") => Some(false),
        _ => None,
    }
}

type XmlErr = quick_xml::Error;

fn xml_err(part: &str, e: XmlErr) -> CoreError {
    CoreError::Xml {
        part: part.into(),
        source: Box::new(e),
    }
}

/// 跳过当前元素的剩余内容（配平同名嵌套）。调用点位于 Start 之后。
fn skip_element(reader: &mut Reader<&[u8]>, buf: &mut Vec<u8>, name: &str) -> CoreResult<()> {
    skip_element_dyn(&mut *reader, buf, name)
}

fn skip_element_dyn<R: std::io::Read>(
    reader: &mut Reader<R>,
    buf: &mut Vec<u8>,
    name: &str,
) -> CoreResult<()> {
    let mut depth = 1usize;
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::Start(e)) if local_name(&e) == name => depth += 1,
            Ok(Event::End(e)) if local_name_end(&e) == name => {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
            Ok(Event::Eof) => {
                return Err(CoreError::Xml {
                    part: "word/document.xml".into(),
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unclosed element",
                    )),
                })
            }
            Ok(_) => {}
            Err(e) => return Err(xml_err("word/document.xml", e)),
        }
        buf.clear();
    }
}

// =========================
// styles.xml
// =========================

/// 解析 styles.xml → (样式表, docDefaults)。
pub fn parse_styles_xml(
    xml: &str,
) -> CoreResult<(HashMap<String, StyleDefinition>, Option<StyleDefinition>)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut styles: HashMap<String, StyleDefinition> = HashMap::new();
    let mut doc_defaults: Option<StyleDefinition> = None;
    // 字符样式 rPr 暂存：供段落样式 `<w:rStyle>` 引用与同名兜底合并。
    let mut char_rprs: HashMap<String, DirectRunProps> = HashMap::new();
    // 段落样式里显式声明的 rStyle 引用：styleId → 引用的字符样式 id。
    let mut para_rstyle_refs: HashMap<String, String> = HashMap::new();

    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/styles.xml", e)),
        };
        match &event {
            Event::Start(e) if local_name(e) == "docDefaults" => {
                let mut def = StyleDefinition {
                    style_id: "@docDefaults".into(),
                    name: None,
                    based_on: None,
                    next_paragraph_style: None,
                    paragraph_props: Default::default(),
                    run_props: Default::default(),
                    is_default: true,
                    aliases: vec![],
                };
                parse_style_body(&mut reader, &mut buf, &mut def, None)?;
                doc_defaults = Some(def);
            }
            Event::Start(e) if local_name(e) == "style" => {
                let id = attr_val(e, "styleId").unwrap_or_default();
                let stype = attr_val(e, "type").unwrap_or_else(|| "paragraph".into());
                if !id.is_empty() {
                    let mut def = StyleDefinition {
                        style_id: id.clone(),
                        name: None,
                        based_on: None,
                        next_paragraph_style: None,
                        paragraph_props: Default::default(),
                        run_props: Default::default(),
                        is_default: false,
                        aliases: vec![],
                    };
                    parse_style_body(&mut reader, &mut buf, &mut def, Some(&mut para_rstyle_refs))?;
                    if stype == "character" {
                        char_rprs.insert(id.clone(), def.run_props.clone());
                    }
                    styles.insert(id, def);
                } else {
                    skip_element(&mut reader, &mut buf, "style")?;
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    // linked/rStyle 联动：把被引用字符样式的 rPr 作为空缺补齐进段落样式。
    for (sid, ref_id) in para_rstyle_refs {
        if let Some(rpr) = char_rprs.get(&ref_id) {
            if let Some(def) = styles.get_mut(&sid) {
                merge_run_defaults(&mut def.run_props, rpr);
                def.aliases.push(format!("rStyle:{ref_id}"));
            }
        }
    }
    Ok((styles, doc_defaults))
}

/// 把 from 中 target 空缺字段补上（linked 合并用）。
fn merge_run_defaults(target: &mut DirectRunProps, from: &DirectRunProps) {
    macro_rules! fill {
        ($($f:ident),*) => { $( if target.$f.is_none() { target.$f = from.$f.clone(); } )* };
    }
    fill!(
        font_size_pt, ascii_font, east_asian_font, hansi_font, cs_font, bold, italic, underline,
        highlight, color
    );
}

/// 解析 `<w:style>` / `<w:docDefaults>` 体直到对应 End：name/basedOn/next/pPr/rPr/rStyle。
fn parse_style_body<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    def: &mut StyleDefinition,
    rstyle_refs: Option<&mut HashMap<String, String>>,
) -> CoreResult<()> {
    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/styles.xml", e)),
        };
        match &event {
            Event::Empty(e) => match local_name(e).as_str() {
                "basedOn" => def.based_on = attr_val(e, "val"),
                "next" => def.next_paragraph_style = attr_val(e, "val"),
                "link" => {
                    if let Some(v) = attr_val(e, "val") {
                        def.aliases.push(format!("linked:{v}"));
                    }
                }
                "outlineLvl" => {
                    if let Some(v) = attr_f64(e, "val") {
                        def.paragraph_props.outline_level = Some(v.max(0.0) as u8);
                    }
                }
                "rStyle" => {
                    if let (Some(v), Some(map)) = (attr_val(e, "val"), rstyle_refs.as_mut()) {
                        map.insert(def.style_id.clone(), v);
                    }
                }
                _ => {}
            },
            Event::Start(e) => match local_name(e).as_str() {
                "name" => {
                    def.name = attr_val(e, "val");
                    skip_element(reader, buf, "name")?;
                }
                "pPr" => {
                    parse_ppr_until_end(reader, buf, &mut def.paragraph_props, rstyle_refs)?;
                }
                "rPr" => {
                    let mut props = DirectRunProps::default();
                    parse_rpr_until_end(reader, buf, &mut props)?;
                    def.run_props = props;
                }
                other => {
                    let n = other.to_string();
                    skip_element(reader, buf, &n)?;
                }
            },
            Event::End(e) => {
                let n = local_name_end(e);
                if n == "style" || n == "docDefaults" {
                    return Ok(());
                }
            }
            Ok(Event::Eof) => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

/// 解析 pPr 内部直到 `</w:pPr>`；顺带捕获其中的 `<w:rStyle>` 引用。
fn parse_ppr_until_end<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    props: &mut DirectParagraphProps,
    rstyle_refs: Option<&mut HashMap<String, String>>,
) -> CoreResult<()> {
    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::End(e) if local_name_end(e) == "pPr" => return Ok(()),
            Event::Empty(e) => handle_ppr_simple(e, props),
            Event::Start(e) => {
                let name = local_name(e);
                match name.as_str() {
                    "rStyle" => {
                        if let (Some(v), Some(map)) = (attr_val(e, "val"), rstyle_refs.as_mut())
                        {
                            // styleId 在 map 外不可知——由调用方补写；这里借 props.style_id 之外
                            // 的通道：pPr 里的 rStyle 极少见，忽略即可。
                            let _ = (v, map);
                        }
                        skip_element(reader, buf, "rStyle")?;
                    }
                    "spacing" | "ind" | "jc" | "outlineLvl" => {
                        handle_ppr_simple(e, props);
                        skip_element(reader, buf, &name)?;
                    }
                    _ => skip_element(reader, buf, &name)?,
                }
            }
            Ok(Event::Eof) => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn handle_ppr_simple(e: &BytesStart<'_>, props: &mut DirectParagraphProps) {
    match local_name(e).as_str() {
        "pStyle" => props.style_id = attr_val(e, "val"),
        "jc" => {
            props.alignment = attr_val(e, "val")
                .as_deref()
                .and_then(Alignment::parse_token)
        }
        "outlineLvl" => {
            if let Some(v) = attr_f64(e, "val") {
                props.outline_level = Some(v.max(0.0) as u8);
            }
        }
        "keepNext" => props.keep_with_next = on_off(attr_val(e, "val")),
        "pageBreakBefore" => props.page_break_before = on_off(attr_val(e, "val")),
        "spacing" => {
            if let Some(line) = attr_f64(e, "line") {
                let rule = attr_val(e, "lineRule").unwrap_or_else(|| "auto".into());
                props.line_spacing = crate::style_resolver::line_spacing_from_ooxml(line as i64, &rule);
            }
        }
        "ind" => {
            if let Some(v) = attr_f64(e, "firstLine") {
                props.first_line_indent = Some(crate::style_resolver::indent_from_twips(v));
            } else if let Some(chars) = attr_f64(e, "firstLineChars") {
                props.first_line_indent = Some(crate::dsl::Indent::Characters {
                    value: chars / 100.0,
                });
            }
            if let Some(v) = attr_f64(e, "left") {
                props.left_indent = Some(crate::style_resolver::indent_from_twips(v));
            }
            if let Some(v) = attr_f64(e, "hanging") {
                props.hanging_indent = Some(crate::style_resolver::indent_from_twips(v));
            }
        }
        _ => {}
    }
}

/// 解析 rPr（run 或样式内通用）直到其 End 事件。
fn parse_rpr_until_end<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    props: &mut DirectRunProps,
) -> CoreResult<()> {
    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::End(e) if local_name_end(e) == "rPr" => return Ok(()),
            Event::Empty(ev) => handle_rpr_simple(ev, props),
            Event::Start(ev) => {
                let name = local_name(ev);
                handle_rpr_simple(ev, props);
                skip_element(reader, buf, &name)?;
            }
            Ok(Event::Eof) => return Ok(()),
            _ => {}
        }
        buf.clear();
    }
}

fn handle_rpr_simple(e: &BytesStart<'_>, props: &mut DirectRunProps) {
    match local_name(e).as_str() {
        "sz" => {
            if let Some(v) = attr_f64(e, "val") {
                props.font_size_pt = Some(half_points_to_pt(v));
            }
        }
        "rFonts" => {
            props.ascii_font = attr_val(e, "ascii");
            props.east_asian_font = attr_val(e, "eastAsia");
            props.hansi_font = attr_val(e, "hAnsi");
            props.cs_font = attr_val(e, "cs");
        }
        "b" => props.bold = on_off(attr_val(e, "val")),
        "i" => props.italic = on_off(attr_val(e, "val")),
        "u" => props.underline = Some(attr_val(e, "val").as_deref() != Some("none")),
        "highlight" => props.highlight = attr_val(e, "val"),
        "color" => props.color = attr_val(e, "val"),
        _ => {}
    }
}

// =========================
// document.xml
// =========================

/// 解析 body：段落 / 表格 / 分节符。返回中文警告列表。
pub fn parse_document_xml(xml: &str, doc: &mut ParsedDocument) -> CoreResult<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let mut section_index = 0usize;
    let mut block_index = 0usize;
    let mut table_depth = 0usize;
    let mut header_footer_depth = 0usize;
    let mut footnote_depth = 0usize;
    let mut page_break_count = 0u32;

    let mut cur_table: Option<TableAcc> = None;

    loop {
        let event = match reader.read_event_into(&mut buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::Start(e) => match local_name(e).as_str() {
                "p" => {
                    let ctx = ParaCtx {
                        in_table: table_depth > 0,
                        in_header_footer: header_footer_depth > 0,
                        in_footnote: footnote_depth > 0,
                        global_index: doc.paragraphs.len(),
                        block_index,
                        section_index,
                    };
                    let (para, breaks) = parse_paragraph(&mut reader, &mut buf, &ctx)?;
                    page_break_count += breaks;
                    push_paragraph_into_doc(doc, &mut cur_table, para, &mut block_index);
                }
                "tbl" => {
                    table_depth += 1;
                    if table_depth == 1 {
                        cur_table = Some(TableAcc::default());
                    }
                }
                "tr" if table_depth == 1 => {
                    if let Some(t) = cur_table.as_mut() {
                        flush_pending_cell(t);
                        t.rows.push(Vec::new());
                    }
                }
                "tc" if table_depth == 1 => {
                    if let Some(t) = cur_table.as_mut() {
                        flush_pending_cell(t);
                        t.pending_cell = Some(CellAcc::default());
                    }
                }
                "sectPr" => {
                    let sect = parse_sectpr(&mut reader, &mut buf, section_index)?;
                    doc.sections.push(sect);
                    section_index += 1;
                }
                "hdr" | "ftr" | "header" | "footer" => header_footer_depth += 1,
                "footnote" | "endnote" => footnote_depth += 1,
                _ => {}
            },
            Event::Empty(e) => match local_name(e).as_str() {
                "p" => {
                    let para = empty_paragraph(doc.paragraphs.len(), block_index, section_index,
                        table_depth > 0, header_footer_depth > 0, footnote_depth > 0);
                    push_paragraph_into_doc(doc, &mut cur_table, para, &mut block_index);
                }
                "br" => {
                    if attr_val(e, "type").as_deref() == Some("page") {
                        page_break_count += 1;
                    }
                }
                _ => {}
            },
            Event::End(e) => match local_name_end(e).as_str() {
                "tbl" if table_depth > 0 => {
                    table_depth -= 1;
                    if table_depth == 0 {
                        if let Some(t) = cur_table.take() {
                            let rows: Vec<TableRow> = t
                                .rows
                                .iter()
                                .enumerate()
                                .map(|(ri, cells)| TableRow {
                                    index: ri,
                                    cells: cells
                                        .iter()
                                        .enumerate()
                                        .map(|(ci, pa)| TableCell {
                                            row_index: ri,
                                            col_index: ci,
                                            paragraph_indices: pa.paragraph_indices.clone(),
                                        })
                                        .collect(),
                                })
                                .collect();
                            let col_count = t.rows.iter().map(|r| r.len()).max().unwrap_or(0);
                            doc.blocks.push(Block::Table(TableBlock {
                                block_index,
                                row_count: rows.len(),
                                col_count,
                                rows,
                                paragraph_indices: t.all_para_indices,
                            }));
                            block_index += 1;
                        }
                    }
                }
                "tr" | "tc" if table_depth == 1 => {
                    if let Some(t) = cur_table.as_mut() {
                        flush_pending_cell(t);
                    }
                }
                "hdr" | "ftr" | "header" | "footer" => {
                    header_footer_depth = header_footer_depth.saturating_sub(1);
                }
                "footnote" | "endnote" => {
                    footnote_depth = footnote_depth.saturating_sub(1);
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    doc.page_break_count = page_break_count;
    if table_depth != 0 {
        warnings.push("文档中的表格结构不完整，部分内容未能检查。".into());
    }
    Ok(warnings)
}

#[derive(Default)]
struct CellAcc {
    paragraph_indices: Vec<usize>,
}
#[derive(Default)]
struct TableAcc {
    rows: Vec<Vec<CellAcc>>,
    pending_cell: Option<CellAcc>,
    all_para_indices: Vec<usize>,
}

fn flush_pending_cell(t: &mut TableAcc) {
    if let Some(cell) = t.pending_cell.take() {
        if let Some(row) = t.rows.last_mut() {
            row.push(cell);
        }
    }
}

fn push_paragraph_into_doc(
    doc: &mut ParsedDocument,
    table: &mut Option<TableAcc>,
    para: Paragraph,
    block_index: &mut usize,
) {
    let gi = para.global_index;
    match table.as_mut() {
        Some(t) => {
            if let Some(c) = t.pending_cell.as_mut() {
                c.paragraph_indices.push(gi);
            }
            t.all_para_indices.push(gi);
        }
        None => {
            doc.blocks.push(Block::Paragraph(gi));
            *block_index += 1;
        }
    }
    doc.paragraphs.push(para);
}

fn empty_paragraph(
    global_index: usize,
    block_index: usize,
    section_index: usize,
    in_table: bool,
    in_hf: bool,
    in_fn: bool,
) -> Paragraph {
    Paragraph {
        global_index,
        block_index,
        section_index,
        text: String::new(),
        runs: vec![],
        direct: DirectParagraphProps::default(),
        style_id: None,
        effective: None,
        role: ParagraphRole::Unknown,
        role_tags: vec![],
        role_confidence: 0.0,
        page_hint: None,
        in_table,
        in_header_footer: in_hf,
        in_footnote_endnote: in_fn,
    }
}

struct ParaCtx {
    in_table: bool,
    in_header_footer: bool,
    in_footnote: bool,
    global_index: usize,
    block_index: usize,
    section_index: usize,
}

/// 解析一个 `<w:p>`（Start 之后）到 End；返回段落与其中新增的分页符数。
fn parse_paragraph<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    ctx: &ParaCtx,
) -> CoreResult<(Paragraph, u32)> {
    let mut direct = DirectParagraphProps::default();
    let mut runs: Vec<Run> = Vec::new();
    let mut text_all = String::new();
    let mut breaks = 0u32;
    let mut summary = TextClassSummary::default();

    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::End(e) if local_name_end(e) == "p" => break,
            Event::Start(e) => match local_name(e).as_str() {
                "pPr" => {
                    parse_ppr_until_end(reader, buf, &mut direct, None)?;
                }
                "r" => {
                    let (run, br) = parse_run(reader, buf, runs.len())?;
                    breaks += br;
                    summary = merge_summary(summary, run.text_class_summary.clone());
                    text_all.push_str(&run.text);
                    runs.push(run);
                }
                _ => {} // hyperlink/smartTag/ins 等容器：其内部 <w:r> 由下一轮循环捕获
            },
            Event::Empty(e) => {
                if local_name(e) == "br" && attr_val(e, "type").as_deref() == Some("page") {
                    breaks += 1;
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    let style_id = direct.style_id.clone();
    Ok((
        Paragraph {
            global_index: ctx.global_index,
            block_index: ctx.block_index,
            section_index: ctx.section_index,
            text: text_all,
            runs,
            direct,
            style_id,
            effective: None,
            role: ParagraphRole::Unknown,
            role_tags: vec![],
            role_confidence: 0.0,
            page_hint: None,
            in_table: ctx.in_table,
            in_header_footer: ctx.in_header_footer,
            in_footnote_endnote: ctx.in_footnote,
        },
        breaks,
    ))
}

fn merge_summary(mut a: TextClassSummary, b: TextClassSummary) -> TextClassSummary {
    a.cjk_count += b.cjk_count;
    a.latin_count += b.latin_count;
    a.digit_count += b.digit_count;
    a.punctuation_count += b.punctuation_count;
    a.symbol_count += b.symbol_count;
    a.whitespace_count += b.whitespace_count;
    a.visible_chars += b.visible_chars;
    a
}

/// 解析一个 `<w:r>`（Start 之后）到 End。
fn parse_run<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    index: usize,
) -> CoreResult<(Run, u32)> {
    let mut props = DirectRunProps::default();
    let mut character_style_id: Option<String> = None;
    let mut inline_objects: Vec<InlineObject> = Vec::new();
    let mut text = String::new();
    let mut breaks = 0u32;

    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::End(e) if local_name_end(e) == "r" => break,
            Event::Start(e) => match local_name(e).as_str() {
                "rPr" => {
                    let mut holder = RprRefs {
                        props: &mut props,
                        character_style_id: &mut character_style_id,
                    };
                    holder.parse_until_rpr_end(reader, buf)?;
                }
                "t" => {
                    if let Ok(Event::Text(t)) = reader.read_event_into(buf) {
                        text.push_str(&t.unescape().unwrap_or_default());
                    }
                    consume_end(reader, buf, "t")?;
                }
                "drawing" => {
                    inline_objects.push(scan_inline_object(reader, buf, "drawing", InlineObjectKind::Drawing)?);
                }
                "pict" => {
                    inline_objects.push(scan_inline_object(reader, buf, "pict", InlineObjectKind::Picture)?);
                }
                "object" => {
                    inline_objects.push(scan_inline_object(reader, buf, "object", InlineObjectKind::OleObject)?);
                }
                other => {
                    let n = other.to_string();
                    skip_element(reader, buf, &n)?;
                }
            },
            Event::Empty(e) => match local_name(e).as_str() {
                "rStyle" => character_style_id = attr_val(e, "val"),
                "br" => {
                    if attr_val(e, "type").as_deref() == Some("page") {
                        breaks += 1;
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    let text_class_summary = crate::dsl::classify_text(&text);
    Ok((
        Run {
            index,
            text,
            direct: props,
            character_style_id,
            effective: None,
            inline_objects,
            text_class_summary,
        },
        breaks,
    ))
}

/// rPr 解析器（run 上下文版）：除常规属性外还捕获 `<w:rStyle>`。
struct RprRefs<'a> {
    props: &'a mut DirectRunProps,
    character_style_id: &'a mut Option<String>,
}

impl RprRefs<'_> {
    fn parse_until_rpr_end<B: std::io::Read>(
        &mut self,
        reader: &mut Reader<B>,
        buf: &mut Vec<u8>,
    ) -> CoreResult<()> {
        loop {
            let event = match reader.read_event_into(buf) {
                Ok(ev) => ev,
                Err(e) => return Err(xml_err("word/document.xml", e)),
            };
            match &event {
                Event::End(e) if local_name_end(e) == "rPr" => return Ok(()),
                Event::Empty(ev) => {
                    if local_name(ev) == "rStyle" {
                        *self.character_style_id = attr_val(ev, "val");
                    } else {
                        handle_rpr_simple(ev, self.props);
                    }
                }
                Event::Start(ev) => {
                    let name = local_name(ev);
                    if name == "rStyle" {
                        *self.character_style_id = attr_val(ev, "val");
                    } else {
                        handle_rpr_simple(ev, self.props);
                    }
                    skip_element(reader, buf, &name)?;
                }
                Ok(Event::Eof) => return Ok(()),
                _ => {}
            }
            buf.clear();
        }
    }
}

fn consume_end<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    name: &str,
) -> CoreResult<()> {
    loop {
        match reader.read_event_into(buf) {
            Ok(Event::End(e)) if local_name_end(&e) == name => return Ok(()),
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(xml_err("word/document.xml", e)),
        }
        buf.clear();
    }
}

/// 扫描 drawing/pict/object 内部：提取 extent 尺寸、blip rel id、对象名。
fn scan_inline_object<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    outer: &str,
    kind: InlineObjectKind,
) -> CoreResult<InlineObject> {
    let mut obj = InlineObject {
        object_id: None,
        kind,
        width_emu: None,
        height_emu: None,
        relation_id: None,
    };
    let outer_owned = outer.to_string();
    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::Start(e) | Event::Empty(e) => match local_name(e).as_str() {
                "extent" => {
                    if obj.width_emu.is_none() {
                        obj.width_emu = attr_f64(e, "cx").map(|v| v as u64);
                        obj.height_emu = attr_f64(e, "cy").map(|v| v as u64);
                    }
                }
                "blip" => {
                    if obj.relation_id.is_none() {
                        obj.relation_id = attr_val(e, "embed");
                    }
                }
                "docPr" => {
                    if obj.object_id.is_none() {
                        obj.object_id = attr_val(e, "name");
                    }
                }
                _ => {}
            },
            Event::End(e) if local_name_end(e) == outer_owned => break,
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(obj)
}

/// 解析 `<w:sectPr>`（Start 之后）到 End。
fn parse_sectpr<B: std::io::Read>(
    reader: &mut Reader<B>,
    buf: &mut Vec<u8>,
    index: usize,
) -> CoreResult<SectionProperties> {
    let mut pg_sz: Option<(i64, i64)> = None;
    let mut mar: HashMap<String, i64> = HashMap::new();
    let mut start: Option<u32> = None;
    let mut restart: Option<bool> = None;
    let mut title_page: Option<bool> = None;
    let mut headers = Vec::new();
    let mut footers = Vec::new();

    loop {
        let event = match reader.read_event_into(buf) {
            Ok(ev) => ev,
            Err(e) => return Err(xml_err("word/document.xml", e)),
        };
        match &event {
            Event::End(e) if local_name_end(e) == "sectPr" => break,
            Event::Empty(e) | Event::Start(e) => match local_name(e).as_str() {
                "pgSz" => {
                    let w = attr_f64(e, "w").unwrap_or(0.0) as i64;
                    let h = attr_f64(e, "h").unwrap_or(0.0) as i64;
                    pg_sz = Some((w, h));
                }
                "pgMar" => {
                    for k in ["top", "bottom", "left", "right", "header", "footer", "gutter"] {
                        if let Some(v) = attr_f64(e, k) {
                            mar.insert(k.to_string(), v as i64);
                        }
                    }
                }
                "pgNumType" => {
                    if let Some(s) = attr_val(e, "start").and_then(|v| v.parse::<u32>().ok()) {
                        start = Some(s);
                    }
                    if let Some(f) = attr_val(e, "fmt") {
                        restart = Some(f != "continue");
                    }
                }
                "titlePg" => title_page = Some(on_off(attr_val(e, "val")).unwrap_or(true)),
                "headerReference" => {
                    if let Some(id) = attr_val(e, "id") {
                        headers.push(id);
                    }
                }
                "footerReference" => {
                    if let Some(id) = attr_val(e, "id") {
                        footers.push(id);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    let get = |k: &str| mar.get(k).copied();
    let margins = if mar.is_empty() {
        None
    } else {
        Some(Margins {
            top_cm: twips_to_cm(get("top").unwrap_or(0)),
            bottom_cm: twips_to_cm(get("bottom").unwrap_or(0)),
            left_cm: twips_to_cm(get("left").unwrap_or(0)),
            right_cm: twips_to_cm(get("right").unwrap_or(0)),
            header_cm: get("header").map(twips_to_cm),
            footer_cm: get("footer").map(twips_to_cm),
            gutter_cm: get("gutter").map(twips_to_cm),
        })
    };
    Ok(SectionProperties {
        index,
        page_size: pg_sz.map(|(w, h)| PageSize {
            width_mm: twips_to_mm(w),
            height_mm: twips_to_mm(h),
        }),
        margins,
        page_number_start: start,
        restart_page_numbering: restart,
        title_page,
        header_references: headers,
        footer_references: footers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("fmt_docx_parser_tests");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    /// 构造一个内存 docx（zip），返回路径；同时清理残留 sidecar。
    fn make_docx(name: &str, parts: &[(&str, &str)]) -> PathBuf {
        let path = tmp(name);
        let file = std::fs::File::create(&path).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (n, content) in parts {
            w.start_file(*n, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        let _ = std::fs::remove_file(sidecar_json(&path));
        path
    }

    const DOC_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

    fn doc_xml(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{DOC_NS}"><w:body>{body}</w:body></w:document>"#
        )
    }

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }

    #[test]
    fn missing_file_is_invalid_path() {
        let e = parse_docx(&tmp("nope_xyz_q1.docx")).unwrap_err();
        assert!(matches!(e, CoreError::InvalidPath { .. }));
    }

    #[test]
    fn non_zip_bytes_report_corrupted() {
        let p = tmp("broken_new_q1.docx");
        std::fs::write(&p, b"this is definitely not a docx").unwrap();
        let e = parse_docx(&p).unwrap_err();
        assert!(matches!(e, CoreError::Zip { .. }));
        let msg = e.user_message();
        assert!(msg.contains("损坏") || msg.contains("密码"));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn real_zip_without_document_reports_missing_part() {
        let p = make_docx("no_doc_part_q1.docx", &[("[Content_Types].xml", "<Types/>")]);
        let e = parse_docx(&p).unwrap_err();
        assert!(matches!(e, CoreError::MissingPart { .. }), "got {e:?}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parses_paragraph_runs_and_direct_format() {
        let body = r#"<w:p><w:pPr><w:pStyle w:val="a"/><w:jc w:val="center"/></w:pPr>
<w:r><w:rPr><w:rFonts w:ascii="Arial" w:eastAsia="黑体"/><w:sz w:val="28"/><w:b/></w:rPr><w:t>标题文本</w:t></w:r>
<w:r><w:t xml:space="preserve"> 正文 tail</w:t></w:r></w:p>"#;
        let p = make_docx("basic_q1.docx", &[("word/document.xml", &doc_xml(body))]);
        let doc = parse_docx(&p).expect("应能解析");
        assert_eq!(doc.paragraphs.len(), 1);
        let para = &doc.paragraphs[0];
        assert_eq!(para.text, "标题文本 正文 tail");
        assert_eq!(para.runs.len(), 2);
        assert_eq!(para.direct.alignment, Some(Alignment::Center));
        assert_eq!(para.direct.style_id.as_deref(), Some("a"));
        let r0 = &para.runs[0];
        assert_eq!(r0.direct.font_size_pt, Some(14.0), "28 半磅 = 14pt");
        assert_eq!(r0.direct.ascii_font.as_deref(), Some("Arial"));
        assert_eq!(r0.direct.east_asian_font.as_deref(), Some("黑体"));
        assert_eq!(r0.direct.bold, Some(true));
        assert!(r0.text_class_summary.cjk_count >= 4);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn effective_format_resolved_from_styles_chain() {
        let styles = format!(
            r#"<?xml version="1.0"?><w:styles xmlns:w="{DOC_NS}">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="宋体"/><w:sz w:val="21"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
</w:styles>"#
        );
        let body = r#"<w:p><w:pPr><w:pStyle w:val="H1"/></w:pPr><w:r><w:t>一级标题</w:t></w:r></w:p>"#;
        let p = make_docx(
            "styled_q1.docx",
            &[("word/document.xml", &doc_xml(body)), ("word/styles.xml", &styles)],
        );
        let doc = parse_docx(&p).expect("应能解析");
        let eff = doc.paragraphs[0].runs[0]
            .effective
            .as_ref()
            .expect("应有有效格式");
        assert_eq!(eff.font_size_pt, Some(16.0), "32 半磅 → 16pt 来自 H1 样式");
        assert!(eff.bold, "加粗应继承自 H1");
        assert_eq!(eff.east_asian_font.as_deref(), Some("宋体"), "docDefaults 补齐字体");
        let src = eff.sources.map.get("font_size_pt").map(String::as_str);
        assert!(src.is_some(), "有效格式应标注来源，got {:?}", eff.sources);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn tables_and_section_properties() {
        let body = r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>甲</w:t></w:r></w:p></w:tc>
<w:tc><w:p><w:r><w:t>乙</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
<w:p><w:r><w:t>表后文字</w:t></w:r><w:r><w:br w:type="page"/></w:r></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1800" w:bottom="1440" w:left="1800" w:header="851" w:footer="992" w:gutter="0"/></w:sectPr>"#;
        let p = make_docx("table_q1.docx", &[("word/document.xml", &doc_xml(body))]);
        let doc = parse_docx(&p).expect("应能解析");
        assert_eq!(doc.paragraphs.len(), 3);
        assert!(doc.paragraphs[0].in_table && doc.paragraphs[1].in_table);
        let tbl = doc
            .blocks
            .iter()
            .find_map(|b| match b {
                Block::Table(t) => Some(t),
                _ => None,
            })
            .expect("应有表格块");
        assert_eq!(tbl.row_count, 1);
        assert_eq!(tbl.col_count, 2);
        assert_eq!(tbl.paragraph_indices, vec![0, 1]);
        assert_eq!(doc.page_break_count, 1);
        let sect = &doc.sections[0];
        let m = sect.margins.as_ref().unwrap();
        assert!((m.top_cm - 2.54).abs() < 0.01, "1440twips≈2.54cm got {}", m.top_cm);
        assert!((m.left_cm - 3.175).abs() < 0.01);
        let ps = sect.page_size.as_ref().unwrap();
        assert!(
            (ps.width_mm - 210.0).abs() < 0.5,
            "A4 宽 11906twips≈210mm got {}",
            ps.width_mm
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn derived_captions_and_citations_extracted_at_parse_time() {
        let body = r#"<w:p><w:r><w:t>本文方法已被验证[1]。</w:t></w:r></w:p>
<w:p><w:r><w:t>图1 系统结构</w:t></w:r></w:p>
<w:p><w:r><w:t>参考文献</w:t></w:r></w:p>
<w:p><w:r><w:t>[1] 张三. 研究[J]. 学报, 2020(1): 3-9.</w:t></w:r></w:p>"#;
        let p = make_docx("derived_q1.docx", &[("word/document.xml", &doc_xml(body))]);
        let doc = parse_docx(&p).expect("应能解析");
        assert_eq!(doc.captions.len(), 1);
        assert_eq!(doc.captions[0].kind, crate::dsl::CaptionKind::Figure);
        assert_eq!(doc.citations.len(), 1);
        assert_eq!(doc.reference_entries.len(), 1);
        assert_eq!(doc.paragraphs[1].role, ParagraphRole::FigureCaption);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn drawings_recorded_as_inline_objects() {
        let body = r#"<w:p><w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:extent cx="914400" cy="457200"/><wp:docPr name="图片 4"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip r:embed="rId5" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></pic:blipFill></pic:pic></a:graphic></wp:inline></w:drawing></w:r><w:r><w:t>图1 说明</w:t></w:r></w:p>"#;
        let p = make_docx("draw_q1.docx", &[("word/document.xml", &doc_xml(body))]);
        let doc = parse_docx(&p).expect("应能解析");
        let objs = &doc.paragraphs[0].runs[0].inline_objects;
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].width_emu, Some(914400));
        assert_eq!(objs[0].relation_id.as_deref(), Some("rId5"));
        assert!(doc.paragraphs[0]
            .role_tags
            .contains(&crate::dsl::RoleTag::HasDrawing));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupted_sidecar_still_maps_to_sidecar_invalid() {
        let p = tmp("sidecar_broken_q1.docx");
        std::fs::write(&p, [b'P', b'K', 3, 4, 0, 0, 0, 0]).unwrap();
        std::fs::write(sidecar_json(&p), b"{ this is not valid json ").unwrap();
        let e = parse_docx(&p).unwrap_err();
        assert!(matches!(e, CoreError::SidecarInvalid { .. }), "got {e:?}");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(sidecar_json(&p));
    }

    #[test]
    fn malformed_xml_returns_friendly_error() {
        let p = make_docx("bad_xml_q1.docx", &[("word/document.xml", "<w:document><w:p>")]);
        let e = parse_docx(&p).unwrap_err();
        assert!(matches!(e, CoreError::Xml { .. }), "got {e:?}");
        assert!(!e.user_message().is_empty());
        let _ = std::fs::remove_file(&p);
    }
}
