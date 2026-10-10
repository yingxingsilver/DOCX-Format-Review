//! dsl —— 规则引擎的权威数据模型（与 rules/rules.json 一一对应）。
//!
//! 设计来源：仓库根目录 `dsl.rs` 草稿 + `rules/rules.json` 实际字段，修正点：
//! - `Checker::check_paragraph(&self, ctx, paragraph: &Paragraph)`（原草稿生命周期写法无法编译）；
//! - 新增 `is_document_level()`：区分文档级规则与段落级规则；
//! - 其余字段命名、serde tag（`type` / `rule` / `unit` / `kind`）与 rules.json 保持一致。
//!
//! 约定：
//! - 所有数值单位在“进入本模型前”完成换算（半磅→pt、twip→mm/cm），DSL 中只出现人类可读单位。
//! - 本模块不含解析逻辑与业务判断，仅数据结构 + 少量纯函数工具。

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// =========================
// 基础枚举
// =========================

/// 严重级别。对普通用户展示为：错误 / 警告 / 提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn label_cn(self) -> &'static str {
        match self {
            Severity::Error => "错误",
            Severity::Warning => "警告",
            Severity::Info => "提示",
        }
    }
}

/// 对齐方式（对应 OOXML w:jc，已归一化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Left,
    Center,
    Right,
    Justify,
    Distribute,
    Unknown,
}

impl Alignment {
    pub fn label_cn(self) -> &'static str {
        match self {
            Alignment::Left => "左对齐",
            Alignment::Center => "居中",
            Alignment::Right => "右对齐",
            Alignment::Justify => "两端对齐",
            Alignment::Distribute => "分散对齐",
            Alignment::Unknown => "未知",
        }
    }

    /// 解析 rules.json 中的位置字符串（如 center / right_bottom）。
    pub fn parse_token(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Some(Self::Left),
            "center" | "centre" => Some(Self::Center),
            "right" => Some(Self::Right),
            "justify" | "both" => Some(Self::Justify),
            "distribute" => Some(Self::Distribute),
            _ => None,
        }
    }
}

/// 行距规则种类（对应 w:lineRule）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineSpacingRuleKind {
    /// 倍数行距：w:line 单位为 1/240 行。
    Auto,
    /// 固定值：w:line 单位为 1/20 磅。
    Exact,
    /// 最小值：w:line 单位为 1/20 磅。
    AtLeast,
}

/// 有效行距（已换算为 pt / 倍数）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum LineSpacing {
    Multiple { factor: f64 },
    Exact { points: f64 },
    AtLeast { points: f64 },
}

/// 缩进（优先字符单位；绝对单位由上层按字号估算为字符）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case")]
pub enum Indent {
    Characters { value: f64 },
    Points { value: f64 },
    Centimeters { value: f64 },
    Twips { value: f64 },
}

impl Indent {
    /// 按正文字号把绝对单位折算为“约等于几个全角字符”。中文全角字符近似宽度 = font_size_pt。
    pub fn approx_chars(&self, font_size_pt: f64) -> Option<f64> {
        if font_size_pt <= 0.0 || !font_size_pt.is_finite() {
            return None;
        }
        let pts = match self {
            Indent::Characters { value } => return Some(*value),
            Indent::Points { value } => *value,
            Indent::Centimeters { value } => value * 28.3464566929,
            Indent::Twips { value } => value / 20.0,
        };
        Some(pts / font_size_pt)
    }
}

/// 页面尺寸，单位 mm（OOXML twip → mm：mm = twips * 25.4 / 1440）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

/// 页边距，单位 cm（OOXML twip → cm：cm = twips / 566.9291338582677）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top_cm: f64,
    pub bottom_cm: f64,
    pub left_cm: f64,
    pub right_cm: f64,
    #[serde(default)]
    pub header_cm: Option<f64>,
    #[serde(default)]
    pub footer_cm: Option<f64>,
    #[serde(default)]
    pub gutter_cm: Option<f64>,
}

/// 文本字符类别（用于字体检查与字数统计）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextClass {
    Cjk,
    Latin,
    Digit,
    Punctuation,
    Symbol,
    Whitespace,
    Other,
}

/// 单个 run 的字符类别统计。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextClassSummary {
    pub cjk_count: usize,
    pub latin_count: usize,
    pub digit_count: usize,
    pub punctuation_count: usize,
    #[serde(default)]
    pub symbol_count: usize,
    pub whitespace_count: usize,
    pub visible_chars: usize,
}

impl TextClassSummary {
    pub fn has_cjk(&self) -> bool {
        self.cjk_count > 0
    }
    pub fn has_latin_or_digit(&self) -> bool {
        self.latin_count > 0 || self.digit_count > 0
    }
}

/// 判断一个 char 是否属于 CJK（含扩展 A/B 及兼容区，够用即可，不做语义分析）。
pub fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x3000..=0x303F | 0xFF00..=0xFFEF)
}

/// 是否 CJK 统一表意文字（不含标点区）。
pub fn is_cjk_ideograph(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// 是否 CJK 语境标点（全角标点、中文标点等）。
pub fn is_cjk_punct(c: char) -> bool {
    ('\u{3000}'..='\u{303F}').contains(&c)
        || ('\u{FF00}'..='\u{FFEF}').contains(&c)
        || matches!(c, '，' | '。' | '；' | '：' | '？' | '！' | '、' | '（' | '）' | '《' | '》' | '“' | '”' | '‘' | '’' | '—' | '…')
}

/// 按字符类别统计一段文本。
pub fn classify_text(text: &str) -> TextClassSummary {
    let mut s = TextClassSummary::default();
    for c in text.chars() {
        if c.is_whitespace() {
            s.whitespace_count += 1;
            continue;
        }
        s.visible_chars += 1;
        if is_cjk_ideograph(c) {
            s.cjk_count += 1;
        } else if c.is_ascii_alphabetic() || (c.is_alphabetic() && !is_cjk_ideograph(c)) {
            s.latin_count += 1;
        } else if c.is_ascii_digit() {
            s.digit_count += 1;
        } else if is_cjk_punct(c) || c.is_ascii_punctuation() {
            s.punctuation_count += 1;
        } else {
            s.symbol_count += 1;
        }
    }
    s
}

// =========================
// DOCX 原始结构与有效格式
// =========================

/// run 直接格式（rPr 归一化后；字号已由半磅换算为 pt）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectRunProps {
    pub font_size_pt: Option<f64>,
    pub ascii_font: Option<String>,
    pub east_asian_font: Option<String>,
    #[serde(default)]
    pub hansi_font: Option<String>,
    #[serde(default)]
    pub cs_font: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub highlight: Option<String>,
    pub color: Option<String>,
}

/// 段落直接格式（pPr 归一化后）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectParagraphProps {
    pub style_id: Option<String>,
    pub alignment: Option<Alignment>,
    pub line_spacing: Option<LineSpacing>,
    pub first_line_indent: Option<Indent>,
    pub left_indent: Option<Indent>,
    pub hanging_indent: Option<Indent>,
    pub outline_level: Option<u8>,
    pub keep_with_next: Option<bool>,
    pub page_break_before: Option<bool>,
}

/// 样式表条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleDefinition {
    pub style_id: String,
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub next_paragraph_style: Option<String>,
    #[serde(default)]
    pub paragraph_props: DirectParagraphProps,
    #[serde(default)]
    pub run_props: DirectRunProps,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// 记录每个属性的取值来源（run_direct / character_style / paragraph_style / doc_defaults）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FormatSources {
    pub map: HashMap<String, String>,
}

/// run 经样式继承后的最终生效格式。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectiveRunFormat {
    pub font_size_pt: Option<f64>,
    pub ascii_font: Option<String>,
    pub east_asian_font: Option<String>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    pub highlight: Option<String>,
    pub color: Option<String>,
    #[serde(default)]
    pub sources: FormatSources,
}

/// 段落经样式继承后的最终生效格式。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectiveParagraphFormat {
    pub alignment: Option<Alignment>,
    pub line_spacing: Option<LineSpacing>,
    pub first_line_indent: Option<Indent>,
    pub left_indent: Option<Indent>,
    pub hanging_indent: Option<Indent>,
    pub outline_level: Option<u8>,
    pub style_id: Option<String>,
    #[serde(default)]
    pub sources: FormatSources,
}

/// run 内嵌对象类型（图、公式等）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InlineObjectKind {
    Drawing,
    Picture,
    OleObject,
    Equation,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InlineObject {
    pub object_id: Option<String>,
    pub kind: InlineObjectKind,
    pub width_emu: Option<u64>,
    pub height_emu: Option<u64>,
    pub relation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub index: usize,
    pub text: String,
    #[serde(default)]
    pub direct: DirectRunProps,
    pub character_style_id: Option<String>,
    /// 样式求解器输出；P2 之前允许为 None（此时相关规则降级为 info/manual_review）。
    pub effective: Option<EffectiveRunFormat>,
    #[serde(default)]
    pub inline_objects: Vec<InlineObject>,
    #[serde(default)]
    pub text_class_summary: TextClassSummary,
}

/// 段落角色（由角色识别器给出；规则引擎只消费，不再猜语义）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParagraphRole {
    Title,
    Author,
    Affiliation,
    AbstractLabel,
    AbstractBody,
    KeywordsLabel,
    KeywordsBody,
    Heading { level: u8 },
    Body,
    FigureCaption,
    TableCaption,
    TableText,
    ReferenceLabel,
    ReferenceEntry,
    AppendixLabel,
    AcknowledgementLabel,
    Other,
    Unknown,
}

impl ParagraphRole {
    /// snake_case 名称，用于与 rules.json 中 target.roles / anchors.roles 字符串匹配。
    pub fn kind_str(&self) -> &'static str {
        match self {
            ParagraphRole::Title => "title",
            ParagraphRole::Author => "author",
            ParagraphRole::Affiliation => "affiliation",
            ParagraphRole::AbstractLabel => "abstract_label",
            ParagraphRole::AbstractBody => "abstract_body",
            ParagraphRole::KeywordsLabel => "keywords_label",
            ParagraphRole::KeywordsBody => "keywords_body",
            ParagraphRole::Heading { .. } => "heading",
            ParagraphRole::Body => "body",
            ParagraphRole::FigureCaption => "figure_caption",
            ParagraphRole::TableCaption => "table_caption",
            ParagraphRole::TableText => "table_text",
            ParagraphRole::ReferenceLabel => "reference_label",
            ParagraphRole::ReferenceEntry => "reference_entry",
            ParagraphRole::AppendixLabel => "appendix_label",
            ParagraphRole::AcknowledgementLabel => "acknowledgement_label",
            ParagraphRole::Other => "other",
            ParagraphRole::Unknown => "unknown",
        }
    }

    pub fn heading_level(&self) -> Option<u8> {
        match self {
            ParagraphRole::Heading { level } => Some(*level),
            _ => None,
        }
    }

    /// 从 rules.json 的角色字符串还原（heading 默认 level 1，由 TargetSpec.level 再过滤）。
    pub fn from_kind_str(s: &str) -> Option<Self> {
        Some(match s {
            "title" => Self::Title,
            "author" => Self::Author,
            "affiliation" => Self::Affiliation,
            "abstract_label" => Self::AbstractLabel,
            "abstract_body" => Self::AbstractBody,
            "keywords_label" => Self::KeywordsLabel,
            "keywords_body" => Self::KeywordsBody,
            "heading" => Self::Heading { level: 1 },
            "body" => Self::Body,
            "figure_caption" => Self::FigureCaption,
            "table_caption" => Self::TableCaption,
            "table_text" => Self::TableText,
            "reference_label" => Self::ReferenceLabel,
            "reference_entry" => Self::ReferenceEntry,
            "appendix_label" => Self::AppendixLabel,
            "acknowledgement_label" => Self::AcknowledgementLabel,
            "other" => Self::Other,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

/// 段落附加标签（邻近对象、引用等结构化事实）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleTag {
    FirstNonEmpty,
    HasDrawing,
    NearImage,
    NearTable,
    InTable,
    InHeader,
    InFooter,
    InFootnote,
    InEndnote,
    ContainsCitation,
    StartsAbstract,
    StartsKeywords,
    StartsReferences,
    ManualReviewRecommended,
}

impl RoleTag {
    pub fn from_str_name(s: &str) -> Option<Self> {
        Some(match s {
            "first_non_empty" => Self::FirstNonEmpty,
            "has_drawing" => Self::HasDrawing,
            "near_image" => Self::NearImage,
            "near_table" => Self::NearTable,
            "in_table" => Self::InTable,
            "in_header" => Self::InHeader,
            "in_footer" => Self::InFooter,
            "in_footnote" => Self::InFootnote,
            "in_endnote" => Self::InEndnote,
            "contains_citation" => Self::ContainsCitation,
            "starts_abstract" => Self::StartsAbstract,
            "starts_keywords" => Self::StartsKeywords,
            "starts_references" => Self::StartsReferences,
            "manual_review_recommended" => Self::ManualReviewRecommended,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub global_index: usize,
    pub block_index: usize,
    pub section_index: usize,
    pub text: String,
    #[serde(default)]
    pub runs: Vec<Run>,
    #[serde(default)]
    pub direct: DirectParagraphProps,
    pub style_id: Option<String>,
    pub effective: Option<EffectiveParagraphFormat>,
    pub role: ParagraphRole,
    #[serde(default)]
    pub role_tags: Vec<RoleTag>,
    #[serde(default)]
    pub role_confidence: f32,
    pub page_hint: Option<u32>,
    #[serde(default)]
    pub in_table: bool,
    #[serde(default)]
    pub in_header_footer: bool,
    #[serde(default)]
    pub in_footnote_endnote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCell {
    pub row_index: usize,
    pub col_index: usize,
    pub paragraph_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRow {
    pub index: usize,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableBlock {
    pub block_index: usize,
    pub row_count: usize,
    pub col_count: usize,
    pub rows: Vec<TableRow>,
    pub paragraph_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Paragraph(usize),
    Table(TableBlock),
}

/// 节属性（页面设置；单位已换算为 mm / cm）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionProperties {
    pub index: usize,
    pub page_size: Option<PageSize>,
    pub margins: Option<Margins>,
    pub page_number_start: Option<u32>,
    pub restart_page_numbering: Option<bool>,
    pub title_page: Option<bool>,
    #[serde(default)]
    pub header_references: Vec<String>,
    #[serde(default)]
    pub footer_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionKind {
    Figure,
    Table,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    AboveObject,
    BelowObject,
    Unknown,
}

/// 解析器识别出的题注（caption 规则直接消费，避免重复扫全文）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionInfo {
    pub paragraph_index: usize,
    pub kind: CaptionKind,
    pub raw_label: String,
    pub number_parts: Vec<u32>,
    pub title_text: Option<String>,
    pub nearby_object_block_index: Option<usize>,
    pub position: CaptionPosition,
    pub confidence: f32,
}

/// 正文中的文内引用（[1] / [1,2] / [1-3]）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitationOccurrence {
    pub paragraph_index: usize,
    pub run_index: Option<usize>,
    pub raw: String,
    pub numbers: Vec<u32>,
    pub page_hint: Option<u32>,
}

/// 文末参考文献条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceEntry {
    pub paragraph_index: usize,
    pub number: u32,
    pub raw: String,
    pub type_tag: Option<String>,
    pub has_year: bool,
    pub has_pages: bool,
    pub has_journal_or_publisher: bool,
}

/// 解析后的文档（规则引擎的唯一输入）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub path: PathBuf,
    pub file_name: String,
    #[serde(default)]
    pub sections: Vec<SectionProperties>,
    #[serde(default)]
    pub blocks: Vec<Block>,
    #[serde(default)]
    pub paragraphs: Vec<Paragraph>,
    #[serde(default)]
    pub styles: HashMap<String, StyleDefinition>,
    pub doc_defaults: Option<StyleDefinition>,
    #[serde(default)]
    pub captions: Vec<CaptionInfo>,
    #[serde(default)]
    pub citations: Vec<CitationOccurrence>,
    #[serde(default)]
    pub reference_entries: Vec<ReferenceEntry>,
    #[serde(default)]
    pub page_break_count: u32,
    #[serde(default)]
    pub parse_warnings: Vec<String>,
}

// =========================
// 规则 DSL
// =========================

/// 容差配置（比较一律用 within()，禁止裸 ==）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    #[serde(default = "default_font_size_tolerance")]
    pub font_size_pt: f64,
    #[serde(default = "default_line_spacing_pt_tolerance")]
    pub line_spacing_pt: f64,
    #[serde(default = "default_line_spacing_multiple_tolerance")]
    pub line_spacing_multiple: f64,
    #[serde(default = "default_margin_cm_tolerance")]
    pub margin_cm: f64,
    #[serde(default = "default_indent_char_tolerance")]
    pub indent_chars: f64,
    #[serde(default = "default_page_mm_tolerance")]
    pub page_mm: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            font_size_pt: default_font_size_tolerance(),
            line_spacing_pt: default_line_spacing_pt_tolerance(),
            line_spacing_multiple: default_line_spacing_multiple_tolerance(),
            margin_cm: default_margin_cm_tolerance(),
            indent_chars: default_indent_char_tolerance(),
            page_mm: default_page_mm_tolerance(),
        }
    }
}

fn default_font_size_tolerance() -> f64 { 0.5 }
fn default_line_spacing_pt_tolerance() -> f64 { 0.2 }
fn default_line_spacing_multiple_tolerance() -> f64 { 0.05 }
fn default_margin_cm_tolerance() -> f64 { 0.1 }
fn default_indent_char_tolerance() -> f64 { 0.2 }
fn default_page_mm_tolerance() -> f64 { 1.0 }

/// 带容差的浮点比较（NaN 一律 false）。
pub fn within(value: f64, target: f64, tol: f64) -> bool {
    value.is_finite() && target.is_finite() && (value - target).abs() <= tol
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleDefaults {
    #[serde(default)]
    pub tolerance: Tolerance,
}

/// run 检查范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunApply {
    AllRuns,
    NonEmptyRuns,
    PrimaryRun,
}

/// 字体检查针对哪一类文本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextClassFilter {
    Cjk,
    Latin,
    Digit,
    LatinOrDigit,
    Auto,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeF64 {
    pub min: f64,
    pub max: f64,
}

/// 行距期望项（allowed 数组元素，满足任意一项即通过）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineSpacingExpectation {
    pub rule: String,
    #[serde(default)]
    pub factor: Option<f64>,
    #[serde(default)]
    pub min_points: Option<f64>,
    #[serde(default)]
    pub max_points: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageNumberingExpectation {
    #[serde(default)]
    pub restart_at_body: bool,
    #[serde(default = "default_one")]
    pub start: u32,
    #[serde(default)]
    pub allowed_positions: Vec<String>,
}

fn default_one() -> u32 { 1 }
fn default_true() -> bool { true }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountMode {
    CjkChars,
    CjkCharsPlusLatinWords,
    VisibleChars,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PunctuationMode {
    PreferFullwidthInCjk,
    ForbidAsciiInCjk,
    ConsistentOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpacingMode {
    Recommend,
    Require,
    Forbid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontListMode {
    Allowlist,
    Blocklist,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptionNumberingRule {
    #[serde(default = "default_one")]
    pub start: u32,
    #[serde(default = "default_true")]
    pub continuous: bool,
    #[serde(default)]
    pub allow_chapter_prefix: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceAnchor {
    pub id: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

/// 规则类型载荷（tag 字段名 `type`，与 rules.json 一致）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleKind {
    PageSize {
        expected: PageSize,
    },
    PageMargins {
        expected_cm: Margins,
    },
    PageNumbering {
        expected: PageNumberingExpectation,
    },
    RunFontSize {
        #[serde(default)]
        expected_pt: Option<f64>,
        #[serde(default)]
        allowed_pt: Option<Vec<f64>>,
        #[serde(default)]
        range_pt: Option<RangeF64>,
        #[serde(default = "default_run_apply")]
        apply_to: RunApply,
    },
    RunFontFamily {
        text_class: TextClassFilter,
        expected_fonts: Vec<String>,
        #[serde(default)]
        blocked_fonts: Option<Vec<String>>,
    },
    LineSpacing {
        allowed: Vec<LineSpacingExpectation>,
    },
    Indent {
        #[serde(default)]
        first_line_chars: Option<f64>,
        #[serde(default)]
        hanging_chars: Option<f64>,
        #[serde(default)]
        left_chars: Option<f64>,
    },
    Alignment {
        expected: Alignment,
    },
    Bold {
        expected: bool,
    },
    HeadingNumbering {
        allowed_schemes: Vec<String>,
        #[serde(default = "default_true")]
        require_consistent: bool,
        #[serde(default)]
        check_level_match: bool,
    },
    Sequence {
        anchors: Vec<SequenceAnchor>,
        #[serde(default)]
        strict: bool,
    },
    TextLength {
        min: usize,
        max: usize,
        count_mode: CountMode,
    },
    KeywordList {
        min_count: usize,
        max_count: usize,
        separators: Vec<String>,
        #[serde(default)]
        allow_trailing_separator: bool,
    },
    Caption {
        caption_kind: CaptionKind,
        pattern: String,
        #[serde(default)]
        expected_alignment: Option<Alignment>,
        #[serde(default)]
        expected_position: Option<CaptionPosition>,
        #[serde(default)]
        numbering: Option<CaptionNumberingRule>,
    },
    ReferenceList {
        entry_pattern: String,
        required_type_tags: Vec<String>,
        #[serde(default = "default_true")]
        check_continuous: bool,
        #[serde(default = "default_true")]
        check_cited: bool,
    },
    CitationOrder {
        #[serde(default = "default_true")]
        require_ascending: bool,
        #[serde(default = "default_true")]
        allow_repeated: bool,
    },
    Punctuation {
        mode: PunctuationMode,
        #[serde(default)]
        exceptions: Vec<String>,
    },
    CjkLatinSpacing {
        mode: SpacingMode,
    },
    FontAllowList {
        mode: FontListMode,
        #[serde(default)]
        allowed_fonts: Option<Vec<String>>,
        #[serde(default)]
        blocked_fonts: Option<Vec<String>>,
    },
}

fn default_run_apply() -> RunApply {
    RunApply::NonEmptyRuns
}

impl RuleKind {
    /// text_length 规则的最小/最大值（供文案展示；非 text_length 返回 0）。
    pub fn length_bounds(&self) -> (usize, usize) {
        match self {
            RuleKind::TextLength { min, max, .. } => (*min, *max),
            _ => (0, 0),
        }
    }
}

impl Rule {
    /// 本规则的长度下限（text_length / keyword_list），其余为 0。
    pub fn kind_min(&self) -> usize {
        match &self.kind {
            RuleKind::TextLength { min, .. } => *min,
            RuleKind::KeywordList { min_count, .. } => *min_count,
            _ => 0,
        }
    }

    /// 本规则的长度上限（text_length / keyword_list），其余为 0。
    pub fn kind_max(&self) -> usize {
        match &self.kind {
            RuleKind::TextLength { max, .. } => *max,
            RuleKind::KeywordList { max_count, .. } => *max_count,
            _ => 0,
        }
    }
}

impl RuleKind {
    /// 文档级规则（一次扫描整篇文档）；其余为段落级规则。
    pub fn is_document_level(&self) -> bool {
        matches!(
            self,
            RuleKind::PageSize { .. }
                | RuleKind::PageMargins { .. }
                | RuleKind::PageNumbering { .. }
                | RuleKind::HeadingNumbering { .. }
                | RuleKind::Sequence { .. }
                | RuleKind::Caption { .. }
                | RuleKind::ReferenceList { .. }
                | RuleKind::CitationOrder { .. }
        )
    }
}

/// 段落匹配条件。roles 为空表示匹配所有段落。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetSpec {
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub level: Option<u8>,
    #[serde(default)]
    pub exclude_roles: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub text_regex: Option<String>,
}

/// 单条规则。message 为面向用户的中文说明；suggestion 为可行动修复建议。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub severity: Severity,
    #[serde(default)]
    pub manual_review: bool,
    pub message: String,
    #[serde(default)]
    pub suggestion: Option<String>,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(flatten)]
    pub kind: RuleKind,
}

/// profile 中对单条规则的覆盖（severity / enabled / manual_review）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleOverride {
    pub id: String,
    #[serde(default)]
    pub severity: Option<Severity>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub manual_review: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileOverride {
    #[serde(default)]
    pub overrides: Vec<RuleOverride>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

/// 角色识别辅助配置（供 P2/P3 解析器使用；规则引擎只透传）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TargetInferenceConfig {
    #[serde(default)]
    pub heading_style_names: Vec<String>,
    #[serde(default)]
    pub title_style_names: Vec<String>,
    #[serde(default)]
    pub abstract_labels: Vec<String>,
    #[serde(default)]
    pub keywords_labels: Vec<String>,
    #[serde(default)]
    pub references_labels: Vec<String>,
    #[serde(default)]
    pub appendix_labels: Vec<String>,
    #[serde(default)]
    pub acknowledgement_labels: Vec<String>,
    pub figure_caption_regex: String,
    pub table_caption_regex: String,
    pub reference_entry_regex: String,
    pub in_text_citation_regex: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FontPolicy {
    #[serde(default)]
    pub allowed_fonts: Vec<String>,
    #[serde(default)]
    pub blocked_decorative_fonts: Vec<String>,
}

/// 规则文件顶层结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConfig {
    #[serde(default)]
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub defaults: RuleDefaults,
    /// 中文字号别名（小四 → 12.0），用于 expected 文案展示。
    #[serde(default)]
    pub font_size_aliases: HashMap<String, f64>,
    pub target_inference: TargetInferenceConfig,
    #[serde(default)]
    pub font_policy: FontPolicy,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileOverride>,
    pub rules: Vec<Rule>,
}

impl RuleConfig {
    pub fn tolerance(&self) -> &Tolerance {
        &self.defaults.tolerance
    }

    /// 找到某个 pt 值的中文别名（如 12.0 → “小四”），找不到返回 None。
    pub fn font_size_alias_cn(&self, pt: f64) -> Option<&str> {
        self.font_size_aliases
            .iter()
            .find(|(_, v)| **v == pt)
            .map(|(k, _)| k.as_str())
    }
}

// =========================
// 审核结果
// =========================

/// 违规位置。文档级问题各字段可为 None。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub section_index: Option<usize>,
    pub block_index: Option<usize>,
    pub paragraph_index: Option<usize>,
    pub run_index: Option<usize>,
    pub page_hint: Option<u32>,
    pub object_ref: Option<String>,
}

/// 一条格式问题。message 必须中文、面向非技术人员；禁止出现 XML 节点名等技术词。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Violation {
    pub rule_id: String,
    pub severity: Severity,
    pub file_path: PathBuf,
    pub location: Location,
    pub message: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub excerpt: Option<String>,
    pub suggestion: Option<String>,
    /// 0.0~1.0，低置信度问题应降级为 info/manual_review。
    pub confidence: f32,
    pub requires_manual_review: bool,
}

/// 单个文件的审核状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Passed,
    NeedsRevision,
    ReadFailed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileAuditSummary {
    pub file_path: PathBuf,
    pub file_name: String,
    pub status: FileStatus,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub violations: Vec<Violation>,
    /// 面向用户的中文失败原因（损坏 docx / 加密 docx / 空文件等），不含技术堆栈。
    pub parse_error: Option<String>,
}

/// 批量审核总报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditReport {
    pub generated_at: String,
    pub input_dir: PathBuf,
    pub rule_file: PathBuf,
    pub rule_profile: Option<String>,
    pub total_files: usize,
    pub passed_files: usize,
    pub failed_files: usize,
    pub read_failed_files: usize,
    pub total_errors: usize,
    pub total_warnings: usize,
    pub total_infos: usize,
    pub files: Vec<FileAuditSummary>,
}

// =========================
// Checker 接口
// =========================

/// 单条规则执行时的上下文。
pub struct AuditContext<'a> {
    pub document: &'a ParsedDocument,
    pub config: &'a RuleConfig,
    pub rule: &'a Rule,
}

/// 规则检查器。必须 Send + Sync 以支持 rayon 并发。
pub trait Checker: Send + Sync {
    fn rule_id(&self) -> &str;

    /// 文档级检查（页面设置、结构顺序、题注编号等，一次扫描）。
    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let _ = ctx;
        Vec::new()
    }

    /// 段落级检查（仅对匹配 target 的段落调用）。
    fn check_paragraph(&self, ctx: &AuditContext<'_>, paragraph: &Paragraph) -> Vec<Violation> {
        let _ = (ctx, paragraph);
        Vec::new()
    }
}

// =========================
// 单元测试
// =========================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_labels_are_chinese() {
        assert_eq!(Severity::Error.label_cn(), "错误");
        assert_eq!(Severity::Warning.label_cn(), "警告");
        assert_eq!(Severity::Info.label_cn(), "提示");
    }

    #[test]
    fn indent_converts_absolute_units_to_chars() {
        // 24 pt ≈ 2 个 12pt 全角字符
        assert!((Indent::Points { value: 24.0 }.approx_chars(12.0).unwrap() - 2.0).abs() < 1e-9);
        // 480 twip = 24 pt ≈ 2 字符
        assert!((Indent::Twips { value: 480.0 }.approx_chars(12.0).unwrap() - 2.0).abs() < 1e-9);
        assert_eq!(Indent::Characters { value: 2.0 }.approx_chars(12.0), Some(2.0));
        assert_eq!(Indent::Points { value: 24.0 }.approx_chars(0.0), None);
    }

    #[test]
    fn role_roundtrip_kind_str() {
        assert_eq!(ParagraphRole::Heading { level: 2 }.kind_str(), "heading");
        assert_eq!(ParagraphRole::from_kind_str("figure_caption"), Some(ParagraphRole::FigureCaption));
        assert_eq!(ParagraphRole::from_kind_str("no_such_role"), None);
    }

    #[test]
    fn within_respects_tolerance_and_nan() {
        assert!(within(12.4, 12.0, 0.5));
        assert!(!within(12.6, 12.0, 0.5));
        assert!(!within(f64::NAN, 12.0, 0.5));
    }

    #[test]
    fn document_level_rules_are_classified() {
        assert!(RuleKind::PageSize { expected: PageSize { width_mm: 210.0, height_mm: 297.0 } }.is_document_level());
        assert!(!RuleKind::RunFontSize {
            expected_pt: Some(12.0), allowed_pt: None, range_pt: None, apply_to: RunApply::NonEmptyRuns
        }.is_document_level());
    }

    #[test]
    fn tolerance_defaults_match_spec() {
        let t = Tolerance::default();
        assert_eq!(t.font_size_pt, 0.5);
        assert_eq!(t.line_spacing_pt, 0.2);
        assert_eq!(t.line_spacing_multiple, 0.05);
        assert_eq!(t.margin_cm, 0.1);
        assert_eq!(t.indent_chars, 0.2);
        assert_eq!(t.page_mm, 1.0);
    }

    #[test]
    fn classify_text_counts_cjk_latin_digits() {
        let s = classify_text("教育AI 2026，公平");
        assert_eq!(s.cjk_count, 5);
        assert!(s.has_latin_or_digit());
        assert!(s.has_cjk());
    }
}
