//! annotated_docx —— 在原 .docx 的 word/document.xml 中为违规文本添加黄色高亮，
//! 产出一份「标注副本」供用户对照修改（P6 进阶项；第一版只做高亮，不做批注）。
//!
//! 设计约定：
//! - 输入是**原始 .docx**（zip + OOXML），不依赖 `ParsedDocument`；本模块用
//!   quick-xml 做一遍轻量定位扫描（w:p / w:r / w:t 文本、w:tbl 嵌套、序号）。
//! - 违规定位优先使用 [`Violation::location`] 中的 block_index / paragraph_index /
//!   run_index；越界时用 excerpt 全文匹配兜底。两者都失败 → 计入 skipped，
//!   **绝不中断整份文档的处理**。
//! - 输出写入新路径，原文件零改动。
//! - 高亮实现：命中 run 的 `<w:rPr>` 内插入 `<w:highlight w:val="yellow"/>`；
//!   run 无 rPr 时新建一个。已有 highlight 的 run 跳过（幂等，可重复运行）。
//! - 全程无 unsafe；任何 zip / XML / IO 错误映射为带中文上下文的 [`CoreError`]。

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use quick_xml::Writer;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::dsl::Violation;
use crate::error::{CoreError, CoreResult};

/// 默认高亮颜色（OOXML ST_HighlightColor 合法取值）。
pub const DEFAULT_HIGHLIGHT_COLOR: &str = "yellow";

/// 单个 document.xml 部件的解压上限（防 OOM）。
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;

/// 一次标注运行的结果统计。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnnotateStats {
    /// 实际加亮的 run 数（不含已有高亮被跳过的）。
    pub highlighted_runs: usize,
    /// 无法定位而跳过的违规数（位置越界且摘录匹配不到）。
    pub skipped_violations: usize,
    /// 命中但已有高亮、保持原样的 run 数（幂等计数）。
    pub already_highlighted: usize,
}

/// 生成标注副本：读取 `source_docx`，对 `violations` 命中的文本 run 加黄色高亮，
/// 写入 `output_path`。原文件不被修改。
///
/// 失败场景（均返回中文友好的 [`CoreError`]，不 panic）：
/// - 源文件不存在 / 不是合法 zip / 缺少 `word/document.xml`；
/// - document.xml 非法 XML；
/// - 文档已加密；输出目录不可写。
pub fn generate_annotated_docx(
    source_docx: &Path,
    violations: &[Violation],
    output_path: &Path,
) -> CoreResult<AnnotateStats> {
    generate_annotated_docx_with_color(
        source_docx,
        violations,
        output_path,
        DEFAULT_HIGHLIGHT_COLOR,
    )
}

/// 同 [`generate_annotated_docx`]，可指定高亮颜色（须为 ST_HighlightColor 取值）。
pub fn generate_annotated_docx_with_color(
    source_docx: &Path,
    violations: &[Violation],
    output_path: &Path,
    color: &str,
) -> CoreResult<AnnotateStats> {
    let src = std::fs::read(source_docx).map_err(|source| CoreError::Io {
        context: format!("打开原始论文文件：{}", source_docx.display()),
        source,
    })?;

    // 1. 读出 word/document.xml（顺带校验 zip 合法性与加密状态）。
    let doc_xml = read_document_part(&src, source_docx)?;

    // 2. 轻量扫描：段落/run 索引与文本表，用于定位与 excerpt 兜底匹配。
    let scan = scan_document(&doc_xml)?;

    // 3. 违规 → 目标 (all_paragraphs 下标, run 下标) 集合；失败的计入 skipped。
    let plan = plan_targets(&scan, violations);

    // 4. 重写 document.xml：命中 run 注入 <w:highlight/>（其余事件原样透传）。
    let (new_xml, done, already) = rewrite_document(&doc_xml, &plan.targets, color)?;

    // 5. 重新打包：逐条目复制，仅替换 word/document.xml。
    repack_zip(&src, "word/document.xml", &new_xml, output_path)?;

    Ok(AnnotateStats {
        highlighted_runs: done,
        skipped_violations: plan.skipped,
        already_highlighted: already,
    })
}

// =========================
// zip 读写
// =========================

fn zip_err(source: zip::result::ZipError) -> CoreError {
    CoreError::Zip {
        note: String::new(),
        source: Box::new(source),
    }
}

/// `Read`/`Write` 适配器：zip 条目解压失败会以 `std::io::Error` 抛出，
/// 统一包装回 [`CoreError::Zip`]（zip crate 官方推荐做法）。
fn io_to_zip_err(source: std::io::Error) -> CoreError {
    zip_err(zip::result::ZipError::Io(source))
}

fn xml_err(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> CoreError {
    CoreError::Xml {
        part: "word/document.xml".into(),
        source: source.into(),
    }
}

/// 未闭合标签错误：quick-xml 在流式解析时遇到 `</x>` 与开标签不匹配才报错，
/// 对“一路正常开标签直到 EOF”的输入不会主动报错（例如
/// `<?xml ...?><w:document><w:body><w:p>`）。因此我们在扫描/改写循环里维护
/// 一个通用深度计数器，EOF 时深度必须归零，否则视为非法 XML，返回与
/// quick-xml 解析失败一致的 [`CoreError::Xml`]（用户文案：“文档内部格式异常”，
/// 见 error.rs 中 CoreError::Xml 的 user_message）。
fn unclosed_tag_err(depth: usize) -> CoreError {
    xml_err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("文档内部格式异常：存在 {} 个未闭合的标签", depth),
    ))
}

fn read_document_part(zip_bytes: &[u8], path: &Path) -> CoreResult<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|source| CoreError::Zip {
        note: format!(
            "{} 可能已损坏或不是有效的 .docx 文档",
            path.display()
        ),
        source: Box::new(source),
    })?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        if entry.name() != "word/document.xml" {
            continue;
        }
        if entry.encrypted() {
            return Err(CoreError::Zip {
                note: "文档已加密，请先在 Word 中取消密码后再进行标注。".into(),
                source: "encrypted document part".into(),
            });
        }
        if entry.size() > MAX_PART_BYTES {
            return Err(CoreError::OutOfRange {
                field: "word/document.xml 解压大小".into(),
                value: format!("{} 字节", entry.size()),
                allowed: format!("不超过 {MAX_PART_BYTES} 字节").into(),
            });
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(io_to_zip_err)?;
        return Ok(buf);
    }
    Err(CoreError::MissingPart {
        part: "word/document.xml".into(),
    })
}

fn repack_zip(
    zip_bytes: &[u8],
    replaced_name: &str,
    replaced_content: &[u8],
    output_path: &Path,
) -> CoreResult<()> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(zip_err)?;

    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| CoreError::Io {
                context: format!("创建标注副本的输出目录 {}", parent.display()),
                source,
            })?;
        }
    }
    let out_file = std::fs::File::create(output_path).map_err(|source| CoreError::Io {
        context: format!("创建标注副本：{}", output_path.display()),
        source,
    })?;
    let mut writer = ZipWriter::new(out_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let name = entry.name().to_string();
        if name == replaced_name {
            continue; // 稍后用新内容统一写入
        }
        if entry.encrypted() {
            return Err(CoreError::Zip {
                note: "文档包含加密内容，请先在 Word 中取消密码后再进行标注。".into(),
                source: "encrypted entry".into(),
            });
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(io_to_zip_err)?;
        // 保留原始压缩方式，避免把已存储条目强行 deflate。
        let per_entry = options.compression_method(entry.compression());
        writer.start_file(name, per_entry).map_err(zip_err)?;
        writer.write_all(&buf).map_err(|source| CoreError::Io {
            context: format!("写出标注副本 {}", output_path.display()),
            source,
        })?;
    }
    writer.start_file(replaced_name, options).map_err(zip_err)?;
    writer
        .write_all(replaced_content)
        .map_err(|source| CoreError::Io {
            context: format!("写出标注副本 {}", output_path.display()),
            source,
        })?;
    writer.finish().map_err(zip_err)?;
    Ok(())
}

// =========================
// document.xml 轻量扫描（定位用）
// =========================

#[derive(Debug, Clone)]
struct RunInfo {
    text: String,
    /// 扫描阶段记录该 run 是否已带 w:highlight；供未来「只加亮未高亮 run」策略使用。
    /// 当前 rewrite_document 用独立状态机做幂等判断，此字段暂不参与决策。
    #[allow(dead_code)]
    has_highlight: bool,
}

#[derive(Debug, Clone)]
struct ParaInfo {
    runs: Vec<RunInfo>,
    in_table: bool,
}

#[derive(Debug, Default)]
struct DocScan {
    /// body 直接子级段落（block_index 语义与此一致）。
    paragraphs: Vec<ParaInfo>,
    /// 全部段落（含表格内部），按出现序排列（paragraph_index 走全局段落表兜底）。
    all_paragraphs: Vec<ParaInfo>,
}

fn scan_document(xml: &[u8]) -> CoreResult<DocScan> {
    let text = std::str::from_utf8(xml).map_err(xml_err)?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;

    let mut scan = DocScan::default();
    let mut table_depth = 0usize; // w:tbl 嵌套层数
    let mut depth = 0usize; // 通用标签深度：EOF 时必须归零，否则存在未闭合标签
    let mut para_runs: Vec<RunInfo> = Vec::new();
    let mut cur_run_text: Option<String> = None;
    let mut run_has_highlight = false;
    let mut in_rpr = false;

    loop {
        let ev = reader.read_event().map_err(xml_err)?;
        match ev {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "tbl" => table_depth += 1,
                    "p" => {
                        para_runs.clear();
                        cur_run_text = None;
                    }
                    "r" => {
                        cur_run_text = Some(String::new());
                        run_has_highlight = false;
                        in_rpr = false;
                    }
                    "rPr" => in_rpr = true,
                    "highlight" if in_rpr => run_has_highlight = true,
                    "t" if cur_run_text.is_some() => {
                        if let Ok(Event::Text(t)) = reader.read_event() {
                            let s = t.unescape().unwrap_or_default().into_owned();
                            if let Some(buf) = cur_run_text.as_mut() {
                                buf.push_str(&s);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let local = local_name(e.name().as_ref());
                if local == "highlight" && in_rpr {
                    run_has_highlight = true;
                }
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let local = local_name(e.name().as_ref());
                match local.as_str() {
                    "tbl" => table_depth = table_depth.saturating_sub(1),
                    "rPr" => in_rpr = false,
                    "r" => {
                        if let Some(t) = cur_run_text.take() {
                            para_runs.push(RunInfo {
                                text: t,
                                has_highlight: run_has_highlight,
                            });
                        }
                    }
                    "p" => {
                        let para = ParaInfo {
                            runs: std::mem::take(&mut para_runs),
                            in_table: table_depth > 0,
                        };
                        if !para.in_table {
                            scan.paragraphs.push(para.clone());
                        }
                        scan.all_paragraphs.push(para);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // 深度断言：EOF 时仍有未闭合标签 → 非法 XML（quick-xml 流式解析对
    // “开标签直到 EOF”不报错，这里补上检出，避免产出损坏的标注副本）。
    if depth != 0 {
        return Err(unclosed_tag_err(depth));
    }
    Ok(scan)
}

fn local_name(qname: &[u8]) -> String {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => String::from_utf8_lossy(&qname[i + 1..]).into_owned(),
        None => String::from_utf8_lossy(qname).into_owned(),
    }
}

// =========================
// 违规定位
// =========================

/// all_paragraphs 下标 → 需要加亮的 run 下标集合（升序去重）。
type Targets = BTreeMap<usize, Vec<usize>>;

#[derive(Debug, Default)]
struct TargetPlan {
    targets: Targets,
    /// 定位失败的违规数。
    skipped: usize,
}

fn plan_targets(scan: &DocScan, violations: &[Violation]) -> TargetPlan {
    let mut plan = TargetPlan::default();
    for v in violations {
        if let Some((para_idx, run_indices)) = locate_one(scan, v) {
            let entry = plan.targets.entry(para_idx).or_default();
            for r in run_indices {
                if !entry.contains(&r) {
                    entry.push(r);
                }
            }
        } else {
            plan.skipped += 1;
        }
    }
    plan
}

/// 单条违规定位：先按索引，越界/失配则 excerpt 兜底。
/// 返回（all_paragraphs 下标, run 下标列表）。
fn locate_one(scan: &DocScan, v: &Violation) -> Option<(usize, Vec<usize>)> {
    // ① 索引定位：block_index → 顶层段落表；paragraph_index → 全局段落表。
    //    两表的下标换算：顶层段落在 all_paragraphs 中的真实下标 = 其前面所有段落的数量。
    if let Some(idx) = map_location_to_all_index(scan, v) {
        if let Some(para) = scan.all_paragraphs.get(idx) {
            let runs = select_runs(para, v.location.run_index);
            if !runs.is_empty() {
                return Some((idx, runs));
            }
        }
    }
    // ② excerpt 兜底：在所有段落里找包含摘录（去空白归一化后）的段落。
    let needle = normalize_for_match(v.excerpt.as_deref().unwrap_or(""));
    if needle.is_empty() {
        return None;
    }
    for (idx, para) in scan.all_paragraphs.iter().enumerate() {
        let joined =
            normalize_for_match(&para.runs.iter().map(|r| r.text.as_str()).collect::<String>());
        if joined.contains(&needle) {
            let runs = select_runs(para, v.location.run_index);
            if !runs.is_empty() {
                return Some((idx, runs));
            }
        }
    }
    None
}

/// 把 Violation.location 映射到 all_paragraphs 下标；越界返回 None。
fn map_location_to_all_index(scan: &DocScan, v: &Violation) -> Option<usize> {
    let loc = &v.location;
    if let Some(bi) = loc.block_index {
        // 顶层段落表第 bi 个 ↔ all_paragraphs 中第 bi 个「非表格」段落的下标。
        let mut seen_top = 0usize;
        for (idx, p) in scan.all_paragraphs.iter().enumerate() {
            if !p.in_table {
                if seen_top == bi {
                    return Some(idx);
                }
                seen_top += 1;
            }
        }
        return None;
    }
    if let Some(pi) = loc.paragraph_index {
        // 解析管线的 global_index 语义 = 正文段落全局序（不含表格内部段落时的等价序）。
        // 若文档没有表格，pi 即 all_paragraphs 下标；有表格时允许 ±容差：先精确取，
        // 取不到再退化为「第 pi 个非表格段落」。
        if let Some(p) = scan.all_paragraphs.get(pi) {
            if !p.in_table || !scan.paragraphs.is_empty() {
                return Some(pi);
            }
        }
        let mut seen_top = 0usize;
        for (idx, p) in scan.all_paragraphs.iter().enumerate() {
            if !p.in_table {
                if seen_top == pi {
                    return Some(idx);
                }
                seen_top += 1;
            }
        }
    }
    None
}

/// 选择段落中要加亮的 run：run_index 命中则只选该 run；否则选所有非空文本 run。
fn select_runs(para: &ParaInfo, run_index: Option<usize>) -> Vec<usize> {
    match run_index {
        Some(ri) => {
            if ri < para.runs.len() && !para.runs[ri].text.trim().is_empty() {
                vec![ri]
            } else {
                Vec::new()
            }
        }
        None => (0..para.runs.len())
            .filter(|&i| !para.runs[i].text.trim().is_empty())
            .collect(),
    }
}

/// excerpt 匹配前的归一化：去掉所有空白（含全角空格），规避 Word 拆 run / 换行差异。
fn normalize_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{3000}')
        .collect()
}

// =========================
// document.xml 改写
// =========================

/// 流式重写：命中目标 run 时在 rPr 关闭前注入 highlight；run 无 rPr 则在 </w:r> 前补建。
/// 其余事件原样写回（命名空间前缀、属性顺序均不变）。
/// 返回 (新 XML, 实际加亮 run 数, 因已有高亮而跳过的 run 数)。
fn rewrite_document(xml: &[u8], targets: &Targets, color: &str) -> CoreResult<(Vec<u8>, usize, usize)> {
    let text = std::str::from_utf8(xml).map_err(xml_err)?;
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    let mut writer = Writer::new(Vec::new());

    // 段落/run 状态机（与 scan_document 同一套计数规则）。
    let mut table_depth = 0usize;
    let mut all_para = 0usize; // 当前 End(p) 对应的 all_paragraphs 下标（进入 p 时记录）
    let mut cur_para_key: Option<usize> = None;
    let mut run_in_para = 0usize;
    let mut active_hit = false; // 当前 run 是否命中目标
    let mut saw_rpr_for_hit = false; // 命中 run 是否已遇到自己的 rPr
    let mut in_rpr_of_hit = false;
    let mut highlighted = 0usize;
    let mut already = 0usize;

    loop {
        let ev = reader.read_event().map_err(xml_err)?;
        match ev {
            Event::Eof => {
                writer.write_event(Event::Eof).map_err(xml_err)?;
                break;
            }
            Event::Start(ref s) if local_name(s.name().as_ref()) == "p" => {
                cur_para_key = Some(all_para);
                all_para += 1;
                run_in_para = 0;
                active_hit = false;
                saw_rpr_for_hit = false;
                in_rpr_of_hit = false;
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::Start(ref s) if local_name(s.name().as_ref()) == "tbl" => {
                table_depth += 1;
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::Start(ref s) if local_name(s.name().as_ref()) == "r" => {
                let hit = cur_para_key
                    .and_then(|k| targets.get(&k))
                    .is_some_and(|runs| runs.contains(&run_in_para));
                run_in_para += 1;
                active_hit = hit;
                saw_rpr_for_hit = false;
                in_rpr_of_hit = false;
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::Start(ref s) if local_name(s.name().as_ref()) == "rPr" => {
                if active_hit {
                    saw_rpr_for_hit = true;
                    in_rpr_of_hit = true;
                }
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::Empty(ref s)
                if active_hit && in_rpr_of_hit && local_name(s.name().as_ref()) == "highlight" =>
            {
                // 已有高亮：幂等跳过。
                already += 1;
                in_rpr_of_hit = false; // 不再需要插入
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::End(ref s) if local_name(s.name().as_ref()) == "rPr" => {
                if in_rpr_of_hit {
                    write_highlight(&mut writer, color)?;
                    highlighted += 1;
                }
                in_rpr_of_hit = false;
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::End(ref s) if local_name(s.name().as_ref()) == "r" => {
                if active_hit && !saw_rpr_for_hit {
                    // run 完全没有 rPr：在 </w:r> 之前补建一个。
                    write_open_rpr(&mut writer)?;
                    write_highlight(&mut writer, color)?;
                    write_close_rpr(&mut writer)?;
                    highlighted += 1;
                }
                active_hit = false;
                writer.write_event(ev).map_err(xml_err)?;
            }
            Event::End(ref s) if local_name(s.name().as_ref()) == "tbl" => {
                table_depth = table_depth.saturating_sub(1);
                writer.write_event(ev).map_err(xml_err)?;
            }
            other => {
                writer.write_event(other).map_err(xml_err)?;
            }
        }
    }
    Ok((writer.into_inner(), highlighted, already))
}

fn write_open_rpr(writer: &mut Writer<Vec<u8>>) -> CoreResult<()> {
    let el = BytesStart::new("w:rPr");
    writer
        .write_event(Event::Start(el))
        .map_err(xml_err)
}

fn write_close_rpr(writer: &mut Writer<Vec<u8>>) -> CoreResult<()> {
    let el = BytesStart::new("w:rPr");
    writer.write_event(Event::End(el.to_end())).map_err(xml_err)
}

fn write_highlight(writer: &mut Writer<Vec<u8>>, color: &str) -> CoreResult<()> {
    let mut hl = BytesStart::new("w:highlight");
    hl.push_attribute(("w:val", color));
    writer.write_event(Event::Empty(hl)).map_err(xml_err)
}

// =========================
// 单元测试
// =========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Location, Severity};
    use std::path::PathBuf;

    /// 最小合法 docx（内存构造，不落盘依赖）。
    fn minimal_docx() -> Vec<u8> {
        let doc = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:body>\
<w:p><w:r><w:t>第一段普通文本</w:t></w:r></w:p>\
<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>本文围绕深度学习展开研究</w:t></w:r><w:r><w:t>，方法有效。</w:t></w:r></w:p>\
<w:p><w:r><w:rPr><w:highlight w:val=\"green\"/></w:rPr><w:t>已有高亮的段落</w:t></w:r></w:p>\
</w:body></w:document>";
        let ct = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
</Types>";
        let rels = "\
<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";
        use std::io::Cursor;
        use zip::write::SimpleFileOptions;
        let mut z = ZipWriter::new(Cursor::new(Vec::new()));
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.start_file("[Content_Types].xml", opt).unwrap();
        z.write_all(ct.as_bytes()).unwrap();
        z.start_file("_rels/.rels", opt).unwrap();
        z.write_all(rels.as_bytes()).unwrap();
        z.start_file("word/document.xml", opt).unwrap();
        z.write_all(doc.as_bytes()).unwrap();
        z.finish().unwrap().into_inner()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fmtcore-annot-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_src(dir: &Path) -> PathBuf {
        let p = dir.join("示例.docx");
        std::fs::write(&p, minimal_docx()).unwrap();
        p
    }

    fn viol(rule: &str, bi: Option<usize>, ri: Option<usize>, excerpt: &str) -> Violation {
        Violation {
            rule_id: rule.into(),
            severity: Severity::Error,
            file_path: PathBuf::from("示例.docx"),
            location: Location {
                block_index: bi,
                paragraph_index: bi,
                run_index: ri,
                ..Default::default()
            },
            message: "字号不符".into(),
            expected: None,
            actual: None,
            excerpt: Some(excerpt.into()),
            suggestion: None,
            confidence: 1.0,
            requires_manual_review: false,
        }
    }

    fn extract_doc_xml(annotated: &[u8]) -> String {
        let mut z = ZipArchive::new(Cursor::new(annotated)).unwrap();
        let mut s = String::new();
        z.by_name("word/document.xml").unwrap().read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn highlights_targeted_run_via_block_index() {
        let dir = tmpdir("hl");
        let src = write_src(&dir);
        let out = dir.join("标注副本.docx");
        // 第二个顶层段落（block_index=1）的第 0 个 run 应被加亮。
        let vs = vec![viol("body-font-size", Some(1), Some(0), "本文围绕深度学习")];
        let stats = generate_annotated_docx(&src, &vs, &out).unwrap();
        assert_eq!(stats.highlighted_runs, 1);
        assert_eq!(stats.skipped_violations, 0);

        let xml = extract_doc_xml(&std::fs::read(&out).unwrap());
        // 命中的 run（含 <w:b/> 的那个）现在有 highlight。
        let frag = xml
            .find("<w:rPr><w:b/><w:highlight w:val=\"yellow\"/></w:rPr>")
            .or_else(|| {
                xml.find("<w:highlight w:val=\"yellow\"/>")
                    .filter(|_| xml.matches("<w:rPr>").count() >= 2)
            });
        assert!(frag.is_some(), "应在命中 run 注入黄色高亮:\n{xml}");
        // 未命中的第一个段落不应有高亮。
        let first_p_end = xml.find("</w:p>").unwrap();
        assert!(
            !xml[..first_p_end].contains("highlight"),
            "未命中段落不应被加亮"
        );
        // 原文件未被修改。
        assert_eq!(std::fs::read(&src).unwrap(), minimal_docx());
    }

    #[test]
    fn creates_rpr_when_run_has_none() {
        let dir = tmpdir("norpr");
        let src = write_src(&dir);
        let out = dir.join("out.docx");
        // 第一个段落唯一 run 没有 rPr。
        let vs = vec![viol("x", Some(0), Some(0), "第一段")];
        let stats = generate_annotated_docx(&src, &vs, &out).unwrap();
        assert_eq!(stats.highlighted_runs, 1);
        let xml = extract_doc_xml(&std::fs::read(&out).unwrap());
        assert!(xml.contains("<w:rPr><w:highlight w:val=\"yellow\"/></w:rPr>"), "{xml}");
    }

    #[test]
    fn excerpt_fallback_when_index_out_of_range() {
        let dir = tmpdir("fallback");
        let src = write_src(&dir);
        let out = dir.join("out.docx");
        // block_index=99 越界，靠 excerpt 定位到第三段“已有高亮的段落”。
        let vs = vec![viol("x", Some(99), None, "已有高亮的段落")];
        let stats = generate_annotated_docx(&src, &vs, &out).unwrap();
        assert_eq!(stats.already_highlighted, 1, "已有绿色高亮应幂等跳过");
        assert_eq!(stats.highlighted_runs, 0);
        let xml = extract_doc_xml(&std::fs::read(&out).unwrap());
        assert!(!xml.contains("yellow"), "不应重复注入黄色");
    }

    #[test]
    fn unlocatable_violation_is_skipped_not_fatal() {
        let dir = tmpdir("skip");
        let src = write_src(&dir);
        let out = dir.join("out.docx");
        let vs = vec![viol("x", Some(7), None, "这段文字根本不存在于文档")];
        let stats = generate_annotated_docx(&src, &vs, &out).unwrap();
        assert_eq!(stats.skipped_violations, 1);
        assert_eq!(stats.highlighted_runs, 0);
        assert!(out.exists(), "即使全部跳过也应产出可用副本");
    }

    #[test]
    fn rerun_is_idempotent() {
        let dir = tmpdir("idem");
        let src = write_src(&dir);
        let mid = dir.join("mid.docx");
        let vs = vec![viol("x", Some(1), Some(0), "本文围绕")];
        let s1 = generate_annotated_docx(&src, &vs, &mid).unwrap();
        assert_eq!(s1.highlighted_runs, 1);
        let out = dir.join("out.docx");
        let s2 = generate_annotated_docx(&mid, &vs, &out).unwrap();
        assert_eq!(s2.highlighted_runs, 0, "第二次运行不应重复加亮");
        assert_eq!(s2.already_highlighted, 1);
    }

    #[test]
    fn corrupt_zip_returns_chinese_error() {
        let dir = tmpdir("corrupt");
        let src = dir.join("坏文件.docx");
        // 注意：b"..." 字节串字面量只允许 ASCII，中文用普通字符串转 bytes。
        std::fs::write(&src, format!("PK\x03\x04 {}", "后面全是垃圾").as_bytes()).unwrap();
        let out = dir.join("out.docx");
        let e = generate_annotated_docx(&src, &[], &out).unwrap_err();
        assert!(matches!(e, CoreError::Zip { .. }), "got {e:?}");
        assert!(e.user_message().contains("Word 文档"), "{}", e.user_message());
    }

    #[test]
    fn missing_document_part_errors() {
        let dir = tmpdir("nopart");
        let src = dir.join("无正文.docx");
        // 合法 zip 但没有 word/document.xml。
        use std::io::Cursor;
        let mut z = ZipWriter::new(Cursor::new(Vec::new()));
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("hello.txt", opt).unwrap();
        z.write_all(b"hi").unwrap();
        std::fs::write(&src, z.finish().unwrap().into_inner()).unwrap();
        let e = generate_annotated_docx(&src, &[], &dir.join("o.docx")).unwrap_err();
        assert!(matches!(e, CoreError::MissingPart { .. }));
        assert!(e.user_message().contains("Word"));
    }

    #[test]
    fn invalid_xml_errors_not_panics() {
        let dir = tmpdir("badxml");
        let src = dir.join("坏XML.docx");
        // document.xml 标签不闭合。
        use std::io::Cursor;
        let mut z = ZipWriter::new(Cursor::new(Vec::new()));
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("word/document.xml", opt).unwrap();
        z.write_all(br#"<?xml version="1.0"?><w:document><w:body><w:p>"#)
            .unwrap();
        std::fs::write(&src, z.finish().unwrap().into_inner()).unwrap();
        let e = generate_annotated_docx(&src, &[], &dir.join("o.docx")).unwrap_err();
        assert!(matches!(e, CoreError::Xml { .. }), "got {e:?}");
    }

    #[test]
    fn scan_counts_tables_correctly() {
        let doc = "\
<w:document xmlns:w=\"x\"><w:body>\
<w:p><w:r><w:t>a</w:t></w:r></w:p>\
<w:tbl><w:tr><w:tc><w:p><w:r><w:t>t</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
<w:p><w:r><w:t>b</w:t></w:r></w:p>\
</w:body></w:document>";
        let scan = scan_document(doc.as_bytes()).unwrap();
        assert_eq!(scan.paragraphs.len(), 2, "顶层段落不含表格内段落");
        assert_eq!(scan.all_paragraphs.len(), 3);
        assert!(scan.all_paragraphs[1].in_table);
    }

    #[test]
    fn normalize_strips_fullwidth_space() {
        assert_eq!(normalize_for_match("图\u{3000}1-1"), "图1-1");
    }
}
