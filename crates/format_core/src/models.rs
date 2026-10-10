//! models —— 核心数据模型（P1）。
//!
//! 本模块只定义“文档长什么样”和“审核发现了什么”的数据结构，
//! 不包含任何 DOCX 解析逻辑（解析在 P2 的 `docx_parser` / `style_resolver`）。
//!
//! 单位约定：
//! - 所有字号统一为 **pt**（OOXML 中 w:sz 以半磅存储，解析层负责换算）。
//! - 页边距、行距、缩进等长度统一为 **pt**。
//! - 分节属性中的原始 OOXML 值保持原语义（twips / 1/240 秒等），并在字段注释中标明。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 判断两个 f64 近似相等（用于测试与容差比较）。
pub(crate) fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ============================================================================
// 枚举
// ============================================================================

/// 问题严重级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// 必须修改的硬性错误。
    Error,
    /// 建议修改的问题。
    Warning,
    /// 仅供参考的信息。
    Info,
}

impl Severity {
    /// 面向非技术人员的中文标签。
    pub fn label_cn(&self) -> &'static str {
        match self {
            Severity::Error => "错误",
            Severity::Warning => "警告",
            Severity::Info => "提示",
        }
    }
}

/// 行距计算方式，对应 OOXML `w:spacing/@w:lineRule`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineSpacingRule {
    /// 单倍行距。
    Single,
    /// 1.5 倍行距。
    OneAndHalf,
    /// 2 倍行距。
    Double,
    /// 多倍行距（倍数存于 `raw_240ths`）。
    Multiple {
        /// OOXML 原始值：240 表示单倍。
        raw_240ths: i32,
    },
    /// 最小值（atLeast：行高至少为指定值）。
    AtLeast,
    /// 固定值（exactly：行高严格等于指定值，配合 line_spacing_pt 检查）。
    Exactly,
}

/// 对齐方式，对应 OOXML `w:jc/@w:val`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justified,
}

/// 页面方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageOrientation {
    Portrait,
    Landscape,
}

/// 规则匹配范围：一条规则适用于哪些段落。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum RuleScope {
    /// 全部正文段落（不含表格内、页眉页脚、脚注尾注）。
    AllBody,
    /// 仅标题段落。
    Headings,
    /// 仅第 N 级标题（1 起）。
    HeadingLevel(u8),
    /// 仅指定样式 ID 的段落。
    StyleId(String),
    /// 正文中的图表题注段落（依据文本特征识别，P4 实现）。
    Captions,
}

// ============================================================================
// 文档级信息
// ============================================================================

/// 一节（`<w:sectPr>`）的页面设置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionProperty {
    /// 页宽，单位 twips（1 pt = 20 twips）。
    pub page_width_twips: Option<i64>,
    /// 页高，单位 twips。
    pub page_height_twips: Option<i64>,
    /// 左/右/上/下页边距，单位 pt（已从 twips 换算）。
    pub margins_pt: [f64; 4],
    /// 页眉距边界，pt。
    pub header_distance_pt: Option<f64>,
    /// 页脚距边界，pt。
    pub footer_distance_pt: Option<f64>,
    pub orientation: Option<PageOrientation>,
    /// 装订线，pt。
    pub gutter_pt: Option<f64>,
    /// 首页不同页眉页脚（titlePg）。
    pub different_first_page: bool,
    /// 页码起始值（pgNumType/start）。
    pub page_number_start: Option<u32>,
    /// 原始 sectPr XML 片段，供兜底检查。
    pub raw_xml: String,
}

impl Default for SectionProperty {
    fn default() -> Self {
        // A4 纸 + Word 默认页边距（左右 3.18cm≈90pt，上下 2.54cm≈72pt）。
        Self {
            page_width_twips: Some(11906),
            page_height_twips: Some(16838),
            margins_pt: [90.0, 90.0, 72.0, 72.0],
            header_distance_pt: None,
            footer_distance_pt: None,
            orientation: Some(PageOrientation::Portrait),
            gutter_pt: None,
            different_first_page: false,
            page_number_start: None,
            raw_xml: String::new(),
        }
    }
}

impl SectionProperty {
    /// 将 twips 换算为 pt。
    pub fn twips_to_pt(twips: i64) -> f64 {
        twips as f64 / 20.0
    }
}

/// 一个 .docx 文件解析后的文档级模型（P2 的 docx_parser 负责填充）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentModel {
    /// 文件完整路径。
    pub file_path: String,
    /// 文件名（含扩展名）。
    pub file_name: String,
    /// 正文段落列表（按文档顺序）。
    pub paragraphs: Vec<Paragraph>,
    /// 节/页面设置；绝大多数论文只有一个元素。
    pub sections: Vec<SectionProperty>,
    /// 样式表：style_id -> Style（含默认样式，`default_style_id` 指向 Normal）。
    pub styles: HashMap<String, Style>,
    /// 默认（Normal）样式 ID，通常为 "Normal"。
    pub default_style_id: Option<String>,
    /// 可选的页码估算（无分页数据时由内容长度推算，仅作定位提示）。
    pub estimated_page_count: Option<u32>,
}

impl DocumentModel {
    /// 构造一个空文档模型（测试与解析器初始化用）。
    pub fn empty(file_path: impl Into<String>) -> Self {
        let file_path = file_path.into();
        let file_name = file_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_string();
        Self {
            file_path,
            file_name,
            paragraphs: Vec::new(),
            sections: vec![SectionProperty::default()],
            styles: HashMap::new(),
            default_style_id: None,
            estimated_page_count: None,
        }
    }
}

// ============================================================================
// 段落与 run
// ============================================================================

/// 段落直接格式（pPr）。字段为 None 表示未显式设置，需沿样式链继承。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParagraphProps {
    /// 段前距，pt。
    pub space_before_pt: Option<f64>,
    /// 段后距，pt。
    pub space_after_pt: Option<f64>,
    pub line_spacing_rule: Option<LineSpacingRule>,
    /// 行距绝对值，pt（当 rule 为固定值/最小值时有意义）。
    pub line_spacing_pt: Option<f64>,
    /// 首行缩进，字符数（w:ind/@w:firstLineChars 优先）。
    pub first_line_indent_chars: Option<f64>,
    /// 首行缩进，pt（w:ind/@w:firstLine，已从 twips 换算）。
    pub first_line_indent_pt: Option<f64>,
    /// 左缩进，pt。
    pub indent_left_pt: Option<f64>,
    /// 右缩进，pt。
    pub indent_right_pt: Option<f64>,
    pub alignment: Option<Alignment>,
    /// 大纲级别（0-8；None 表示正文级）。
    pub outline_level: Option<u8>,
    /// 原始 pPr XML 片段，供兜底检查与调试。
    pub raw_xml: Option<String>,
}

/// 字符直接格式（rPr）。字号已统一为 pt（OOXML 半磅值除以 2）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunProps {
    /// 字号，pt。
    pub font_size_pt: Option<f64>,
    /// 西文字体（w:rFonts/@w:ascii）。
    pub ascii_font: Option<String>,
    /// 中文字体（w:rFonts/@w:eastAsia）。
    pub east_asian_font: Option<String>,
    /// 主题字体（w:rFonts/@w:asciiTheme 等原文），供解析层做主题映射。
    pub theme_font: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    /// 高亮颜色（w:highlight/@w:val 原文，如 "yellow"）。
    pub highlight: Option<String>,
    /// 文字颜色（w:color/@w:val，如 "auto"、"FF0000"）。
    pub color: Option<String>,
    /// 删除线。
    pub strike: Option<bool>,
    /// 上标/下标类型（superscript/subscript 原文）。
    pub vert_align: Option<String>,
    /// 原始 rPr XML 片段。
    pub raw_xml: Option<String>,
}

impl Default for RunProps {
    fn default() -> Self {
        Self {
            font_size_pt: None,
            ascii_font: None,
            east_asian_font: None,
            theme_font: None,
            bold: None,
            italic: None,
            underline: None,
            highlight: None,
            color: None,
            strike: None,
            vert_align: None,
            raw_xml: None,
        }
    }
}

impl RunProps {
    /// OOXML w:sz 以半磅存储，此函数是模型层唯一的字号单位换算入口。
    pub fn half_points_to_pt(half_points: f64) -> f64 {
        half_points / 2.0
    }
}

/// 文本 run：一段格式一致的连续文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// run 文本内容。
    pub text: String,
    /// 该 run 的直接格式（rPr）。
    pub direct_format: RunProps,
    /// 显式应用的字符样式 ID（w:rStyle）。
    pub character_style_id: Option<String>,
}

impl Run {
    /// 构造一个仅有文本的 run（其余走样式继承）。
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            direct_format: RunProps::default(),
            character_style_id: None,
        }
    }

    /// 便捷访问：直接格式中的字号（pt）。
    pub fn font_size_pt(&self) -> Option<f64> {
        self.direct_format.font_size_pt
    }

    /// 便捷访问：中文字体。
    pub fn east_asian_font(&self) -> Option<&str> {
        self.direct_format.east_asian_font.as_deref()
    }

    /// 便捷访问：西文字体。
    pub fn ascii_font(&self) -> Option<&str> {
        self.direct_format.ascii_font.as_deref()
    }
}

/// 段落来源位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParagraphSource {
    /// 正文。
    Body,
    /// 表格单元格内。
    Table,
    /// 页眉。
    Header,
    /// 页脚。
    Footer,
    /// 脚注。
    Footnote,
    /// 尾注。
    Endnote,
}

/// 一个段落。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    /// 在文档序列中的索引（0 起）。
    pub index: usize,
    /// 段落样式 ID（w:pStyle）。
    pub style_id: Option<String>,
    /// 段落直接格式（pPr）。
    pub props: ParagraphProps,
    /// run 列表。
    pub runs: Vec<Run>,
    /// 是否被判定为标题（依据 outlineLvl 或样式名，P2 填充）。
    pub is_heading: bool,
    /// 标题层级（1 起；非标题为 None）。
    pub heading_level: Option<u8>,
    /// 是否位于表格单元格内。
    pub in_table: bool,
    /// 是否位于页眉/页脚。
    pub in_header_footer: bool,
    /// 是否位于脚注/尾注。
    pub in_footnote_endnote: bool,
    /// 段落来源位置。
    pub source: ParagraphSource,
    /// 纯文本内容（所有 run 拼接）。
    pub text: String,
}

impl Paragraph {
    /// 构造一个最简正文段落（测试与解析器初始化用）。
    pub fn body(index: usize, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            index,
            style_id: None,
            props: ParagraphProps::default(),
            runs: vec![Run::plain(text.clone())],
            is_heading: false,
            heading_level: None,
            in_table: false,
            in_header_footer: false,
            in_footnote_endnote: false,
            source: ParagraphSource::Body,
            text,
        }
    }
}

// ============================================================================
// 样式
// ============================================================================

/// 样式种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleKind {
    Paragraph,
    Character,
    Table,
    Numbering,
}

/// styles.xml 中的一个样式定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Style {
    /// w:style/@w:styleId。
    pub style_id: String,
    /// w:name/@w:val（可能为中文名，如“标题 1”）。
    pub name: String,
    /// 基于哪个样式（w:basedOn）。
    pub based_on: Option<String>,
    /// 该样式段落之后的下一段落采用的样式（w:next）。
    pub next_paragraph_style: Option<String>,
    /// linked 字符样式 ID（w:link）。
    pub linked_character_style: Option<String>,
    pub kind: StyleKind,
    /// 样式自带的段落属性。
    pub paragraph_props: ParagraphProps,
    /// 样式自带的 run 属性。
    pub run_props: RunProps,
    /// 是否为默认样式（Normal / w:default="1" 的段落样式）。
    pub is_default: bool,
    /// 是否为标题样式（名称匹配“标题 N”或自带 outlineLvl）。
    pub is_heading: bool,
    /// 标题层级（is_heading 时有意义，1 起）。
    pub heading_level: Option<u8>,
}

impl Style {
    /// 构造一个最简段落样式（测试用）。
    pub fn paragraph(style_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            style_id: style_id.into(),
            name: name.into(),
            based_on: None,
            next_paragraph_style: None,
            linked_character_style: None,
            kind: StyleKind::Paragraph,
            paragraph_props: ParagraphProps::default(),
            run_props: RunProps::default(),
            is_default: false,
            is_heading: false,
            heading_level: None,
        }
    }
}

// ============================================================================
// 有效格式与审核结果
// ============================================================================

/// 一个 run 经过样式继承链合并后**最终生效**的格式。
/// 优先级：run 直接格式 > 字符样式 > 段落样式（含 basedOn 链）> docDefaults。
/// 由 P2 的 `style_resolver` 计算产出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveFormat {
    pub font_size_pt: Option<f64>,
    pub ascii_font: Option<String>,
    pub east_asian_font: Option<String>,
    /// 加粗（三态：None 表示整条继承链都未设置）。
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub line_spacing_rule: Option<LineSpacingRule>,
    pub line_spacing_pt: Option<f64>,
    pub first_line_indent_chars: Option<f64>,
}

impl EffectiveFormat {
    /// 全空的有效格式（解析器逐层填充的起点）。
    pub fn empty() -> Self {
        Self {
            font_size_pt: None,
            ascii_font: None,
            east_asian_font: None,
            bold: None,
            italic: None,
            underline: None,
            line_spacing_rule: None,
            line_spacing_pt: None,
            first_line_indent_chars: None,
        }
    }
}

/// 一条审核发现的问题，字段全部面向非技术人员可读。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    /// 触发规则的 ID。
    pub rule_id: String,
    pub severity: Severity,
    /// 所在文件路径。
    pub file_path: String,
    /// 所在段落索引。
    pub paragraph_index: usize,
    /// 所在 run 索引（段落级问题为 None）。
    pub run_index: Option<usize>,
    /// 页码提示（真实分页或估算值）。
    pub page_hint: Option<u32>,
    /// 给用户看的中文描述（不含术语缩写）。
    pub message_for_user: String,
    /// 期望值描述，如“小四（12 磅）”。
    pub expected: String,
    /// 实际值描述，如“五号（10.5 磅）”。
    pub actual: String,
    /// 命中的文本摘录（截断至合理长度）。
    pub excerpt: String,
}

/// 汇总计数。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SummaryCounts {
    pub error_count: u32,
    pub warning_count: u32,
    pub info_count: u32,
}

impl SummaryCounts {
    /// 从违规列表统计各级别数量。
    pub fn from_violations<'a>(violations: impl Iterator<Item = &'a Violation>) -> Self {
        let mut counts = Self::default();
        for v in violations {
            match v.severity {
                Severity::Error => counts.error_count += 1,
                Severity::Warning => counts.warning_count += 1,
                Severity::Info => counts.info_count += 1,
            }
        }
        counts
    }

    pub fn total(&self) -> u32 {
        self.error_count + self.warning_count + self.info_count
    }
}

/// 一次批量审核的总报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditReport {
    /// 生成时间（RFC 3339 字符串，避免引入 chrono 依赖）。
    pub generated_at: String,
    /// 输入目录。
    pub input_dir: String,
    /// 使用的规则文件路径。
    pub rule_file: String,
    pub total_files: u32,
    /// 无 Error 级问题的文件数。
    pub passed_files: u32,
    /// 有至少一个 Error 级问题的文件数。
    pub failed_files: u32,
    /// 每个文件的违规列表（键为文件路径）。
    pub violations_by_file: HashMap<String, Vec<Violation>>,
    pub summary_counts: SummaryCounts,
}

// ============================================================================
// 规则配置 DSL（静态枚举，不做动态脚本引擎）
// ============================================================================

/// 规则文件顶层结构（JSON/YAML 皆可）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConfig {
    /// 规范名称，如“本科毕业论文格式规范”。
    pub name: String,
    #[serde(default)]
    pub tolerance: Tolerance,
    pub rules: Vec<RuleDef>,
}

/// 数值容差（浮点比较时使用）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    /// 字号容差，pt。
    #[serde(default = "default_font_size_tolerance")]
    pub font_size_pt: f64,
    /// 行距容差，pt。
    #[serde(default = "default_line_spacing_tolerance")]
    pub line_spacing_pt: f64,
}

fn default_font_size_tolerance() -> f64 {
    0.5
}

fn default_line_spacing_tolerance() -> f64 {
    0.2
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            font_size_pt: default_font_size_tolerance(),
            line_spacing_pt: default_line_spacing_tolerance(),
        }
    }
}

/// 带容差的数值比较。
pub fn within(value: f64, target: f64, tol: f64) -> bool {
    (value - target).abs() <= tol
}

/// 字体族规则参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFamilySpec {
    /// 要求的中文字体（eastAsia），None 表示不检查。
    pub east_asian: Option<String>,
    /// 要求的西文字体（ascii），None 表示不检查。
    pub ascii: Option<String>,
    /// 允许的替代字体。
    #[serde(default)]
    pub allow_substitutes: Vec<String>,
}

/// 标题样式规则参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadingExpectation {
    /// 层级（1 起）。
    pub level: u8,
    /// 期望的样式 ID 或样式名（二者任一匹配即通过）。
    pub style: String,
    /// 可选：对该级标题额外的格式要求，None 表示不检查。
    pub font_size_pt: Option<f64>,
    pub east_asian_font: Option<String>,
    pub bold: Option<bool>,
}

/// 页边距规则参数，单位 pt；None 表示该项不检查。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarginSpec {
    pub left_pt: Option<f64>,
    pub right_pt: Option<f64>,
    pub top_pt: Option<f64>,
    pub bottom_pt: Option<f64>,
    pub tolerance_pt: Option<f64>,
}

/// 标点规则参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PunctuationSpec {
    /// 禁止出现的字符（英文标点混入中文语境等）。
    #[serde(default)]
    pub forbidden: Vec<String>,
    /// 检查范围说明文本（预留）。
    #[serde(default)]
    pub note: Option<String>,
}

/// 图表编号方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptionNumbering {
    /// 章-序号（图1-1）。
    ChapterDash,
    /// 全文顺序（图1）。
    Sequential,
}

/// 图表编号规则参数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionNumberSpec {
    /// 题注前缀，如“图”“表”。
    pub prefix: String,
    /// 编号模式。
    pub numbering: CaptionNumbering,
    /// 是否要求题注居中。
    #[serde(default)]
    pub require_centered: bool,
}

/// 单条规则定义。JSON 表示：`{ "id": "...", "severity": "...", "applies_to": {...}, "type": "...", ... }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDef {
    /// 规则唯一 ID，出现在 Violation.rule_id 中。
    pub id: String,
    /// 面向用户的中文说明（可选，缺省时由引擎按类型生成模板）。
    pub description: Option<String>,
    pub severity: Severity,
    pub applies_to: RuleScope,
    #[serde(flatten)]
    pub body: RuleBody,
}

/// 规则类型载荷（tag 放在 `type` 字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleBody {
    /// 字号规则（pt，比较使用 tolerance.font_size_pt）。
    FontSize { expected_pt: f64 },
    /// 字体族规则。
    FontFamily { fonts: FontFamilySpec },
    /// 行距规则：rule 与 expected_pt 至少给一个。
    LineSpacing {
        rule: Option<LineSpacingRule>,
        expected_pt: Option<f64>,
    },
    /// 加粗规则。
    Bold { expected: bool },
    /// 斜体规则。
    Italic { expected: bool },
    /// 标题样式规则。
    HeadingStyle { expected: HeadingExpectation },
    /// 页边距规则（作用于节属性）。
    Margin { margins: MarginSpec },
    /// 标点规则。
    Punctuation { spec: PunctuationSpec },
    /// 图表编号规则。
    CaptionNumber { spec: CaptionNumberSpec },
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_ok(a: f64, b: f64) {
        assert!(approx_eq(a, b), "{a} != {b}");
    }

    #[test]
    fn severity_has_three_levels_and_chinese_labels() {
        assert_eq!(Severity::Error.label_cn(), "错误");
        assert_eq!(Severity::Warning.label_cn(), "警告");
        assert_eq!(Severity::Info.label_cn(), "提示");
        assert_ne!(Severity::Error, Severity::Info);
    }

    #[test]
    fn document_model_extracts_file_name_from_path() {
        let doc = DocumentModel::empty("/home/user/论文/thesis_final.docx");
        assert_eq!(doc.file_name, "thesis_final.docx");
        assert!(doc.paragraphs.is_empty());
        assert_eq!(doc.sections.len(), 1);
        let win = DocumentModel::empty("C:\\papers\\a.docx");
        assert_eq!(win.file_name, "a.docx");
    }

    #[test]
    fn section_property_converts_twips_to_pt() {
        approx_ok(SectionProperty::twips_to_pt(1800), 90.0);
        approx_ok(SectionProperty::default().margins_pt[0], 90.0);
    }

    #[test]
    fn run_props_convert_half_points_to_pt() {
        // OOXML 中小四 = 24 半磅 = 12 pt；三号 = 32 半磅 = 16 pt。
        approx_ok(RunProps::half_points_to_pt(24.0), 12.0);
        approx_ok(RunProps::half_points_to_pt(32.0), 16.0);
    }

    #[test]
    fn paragraph_body_constructor_fills_defaults() {
        let p = Paragraph::body(3, "第一段文字");
        assert_eq!(p.index, 3);
        assert_eq!(p.text, "第一段文字");
        assert_eq!(p.runs.len(), 1);
        assert_eq!(p.source, ParagraphSource::Body);
        assert!(!p.is_heading);
        assert_eq!(p.heading_level, None);
        assert!(!p.in_table);
        assert!(!p.in_header_footer);
        assert!(!p.in_footnote_endnote);
    }

    #[test]
    fn run_accessors_read_direct_format() {
        let mut r = Run::plain("hello");
        r.direct_format.font_size_pt = Some(10.5);
        r.direct_format.east_asian_font = Some("宋体".into());
        approx_ok(r.font_size_pt().unwrap(), 10.5);
        assert_eq!(r.east_asian_font(), Some("宋体"));
        assert_eq!(r.ascii_font(), None);
    }

    #[test]
    fn style_paragraph_constructor_is_not_heading_or_default() {
        let s = Style::paragraph("Heading1", "标题 1");
        assert_eq!(s.style_id, "Heading1");
        assert_eq!(s.kind, StyleKind::Paragraph);
        assert!(!s.is_default);
        assert!(!s.is_heading);
        assert_eq!(s.based_on, None);
    }

    #[test]
    fn effective_format_starts_empty() {
        let ef = EffectiveFormat::empty();
        assert_eq!(ef.font_size_pt, None);
        assert_eq!(ef.bold, None);
        assert_eq!(ef.line_spacing_rule, None);
        assert_eq!(ef.first_line_indent_chars, None);
    }

    #[test]
    fn violation_round_trips_json() {
        let v = Violation {
            rule_id: "body-font-size".into(),
            severity: Severity::Error,
            file_path: "a.docx".into(),
            paragraph_index: 12,
            run_index: Some(0),
            page_hint: Some(3),
            message_for_user: "正文字号不符合要求".into(),
            expected: "小四（12 磅）".into(),
            actual: "五号（10.5 磅）".into(),
            excerpt: "本文研究了……".into(),
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: Violation = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn summary_counts_tally_by_severity() {
        let mk = |sev: Severity| Violation {
            rule_id: "r".into(),
            severity: sev,
            file_path: "a.docx".into(),
            paragraph_index: 0,
            run_index: None,
            page_hint: None,
            message_for_user: String::new(),
            expected: String::new(),
            actual: String::new(),
            excerpt: String::new(),
        };
        let vs = vec![
            mk(Severity::Error),
            mk(Severity::Error),
            mk(Severity::Warning),
            mk(Severity::Info),
        ];
        let c = SummaryCounts::from_violations(vs.iter());
        assert_eq!((c.error_count, c.warning_count, c.info_count), (2, 1, 1));
        assert_eq!(c.total(), 4);
    }

    #[test]
    fn audit_report_round_trips_json() {
        let report = AuditReport {
            generated_at: "2026-10-10T08:00:00+00:00".into(),
            input_dir: "/data/papers".into(),
            rule_file: "rules/default.json".into(),
            total_files: 2,
            passed_files: 1,
            failed_files: 1,
            violations_by_file: HashMap::from([(
                "/data/papers/a.docx".to_string(),
                vec![Violation {
                    rule_id: "body-font-size".into(),
                    severity: Severity::Error,
                    file_path: "/data/papers/a.docx".into(),
                    paragraph_index: 12,
                    run_index: Some(0),
                    page_hint: Some(3),
                    message_for_user: "正文字号不符合要求".into(),
                    expected: "小四（12 磅）".into(),
                    actual: "五号（10.5 磅）".into(),
                    excerpt: "本文研究了……".into(),
                }],
            )]),
            summary_counts: SummaryCounts {
                error_count: 1,
                warning_count: 0,
                info_count: 0,
            },
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let back: AuditReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
    }

    #[test]
    fn rule_config_parses_representative_json() {
        let json = r#"{
            "name": "本科毕业论文格式规范",
            "tolerance": { "font_size_pt": 0.5, "line_spacing_pt": 0.2 },
            "rules": [
                { "id": "body-size", "severity": "error", "applies_to": { "kind": "all_body" },
                  "type": "font_size", "expected_pt": 12.0 },
                { "id": "h1-style", "severity": "error", "applies_to": { "kind": "heading_level", "value": 1 },
                  "type": "heading_style",
                  "expected": { "level": 1, "style": "Heading1", "font_size_pt": 16.0,
                                "east_asian_font": "黑体", "bold": true } },
                { "id": "body-spacing", "severity": "warning", "applies_to": { "kind": "all_body" },
                  "type": "line_spacing", "rule": "exactly", "expected_pt": 20.0 },
                { "id": "body-font", "severity": "error", "applies_to": { "kind": "all_body" },
                  "type": "font_family", "fonts": { "east_asian": "宋体", "ascii": "Times New Roman",
                                                    "allow_substitutes": [] } }
            ]
        }"#;
        let cfg: RuleConfig = serde_json::from_str(json).expect("parse rule config");
        assert_eq!(cfg.rules.len(), 4);
        approx_ok(cfg.tolerance.font_size_pt, 0.5);
        match &cfg.rules[0].body {
            RuleBody::FontSize { expected_pt } => approx_ok(*expected_pt, 12.0),
            other => panic!("unexpected rule body: {other:?}"),
        }
        assert_eq!(cfg.rules[1].applies_to, RuleScope::HeadingLevel(1));
        // 序列化回去再解析，保证 DSL 双向稳定。
        let back: RuleConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn margin_punctuation_caption_bold_italic_bodies_deserialize() {
        let json = r#"[
            { "id": "margins", "severity": "error", "applies_to": { "kind": "all_body" },
              "type": "margin", "margins": { "left_pt": 90.0, "right_pt": 90.0, "top_pt": 72.0,
              "bottom_pt": 72.0, "tolerance_pt": 2.0 } },
            { "id": "punct", "severity": "warning", "applies_to": { "kind": "all_body" },
              "type": "punctuation", "spec": { "forbidden": [",", ";"] } },
            { "id": "fig-cap", "severity": "error", "applies_to": { "kind": "captions" },
              "type": "caption_number", "spec": { "prefix": "图", "numbering": "chapter-dash",
              "require_centered": true } },
            { "id": "h1-bold", "severity": "error", "applies_to": { "kind": "headings" },
              "type": "bold", "expected": true },
            { "id": "no-italic-body", "severity": "warning", "applies_to": { "kind": "all_body" },
              "type": "italic", "expected": false }
        ]"#;
        let rules: Vec<RuleDef> = serde_json::from_str(json).expect("parse");
        assert!(
            matches!(&rules[0].body, RuleBody::Margin { margins } if approx_eq(margins.left_pt.unwrap(), 90.0))
        );
        assert!(
            matches!(&rules[1].body, RuleBody::Punctuation { spec } if spec.forbidden.len() == 2)
        );
        assert!(
            matches!(&rules[2].body, RuleBody::CaptionNumber { spec } if spec.numbering == CaptionNumbering::ChapterDash && spec.require_centered)
        );
        assert!(matches!(&rules[3].body, RuleBody::Bold { expected: true }));
        assert!(matches!(
            &rules[4].body,
            RuleBody::Italic { expected: false }
        ));
    }

    #[test]
    fn within_respects_tolerance() {
        assert!(within(12.4, 12.0, 0.5));
        assert!(within(12.5, 12.0, 0.5));
        assert!(!within(12.6, 12.0, 0.5));
    }

    #[test]
    fn document_model_with_paragraphs_round_trips() {
        let mut doc = DocumentModel::empty("/tmp/t.docx");
        doc.default_style_id = Some("Normal".into());
        doc.styles
            .insert("Normal".into(), Style::paragraph("Normal", "正文"));
        doc.paragraphs.push(Paragraph::body(0, "绪论"));
        let json = serde_json::to_string(&doc).unwrap();
        let back: DocumentModel = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }
}
