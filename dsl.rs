use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// =========================
// 基础枚举
// =========================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineSpacingRuleKind {
    Auto,
    Exact,
    AtLeast,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case")]
pub enum LineSpacing {
    Multiple { factor: f64 },
    Exact { points: f64 },
    AtLeast { points: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "unit", rename_all = "snake_case")]
pub enum Indent {
    Characters { value: f64 },
    Points { value: f64 },
    Centimeters { value: f64 },
    Twips { value: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top_cm: f64,
    pub bottom_cm: f64,
    pub left_cm: f64,
    pub right_cm: f64,
    pub header_cm: Option<f64>,
    pub footer_cm: Option<f64>,
    pub gutter_cm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextClassSummary {
    pub cjk_count: usize,
    pub latin_count: usize,
    pub digit_count: usize,
    pub punctuation_count: usize,
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

// =========================
// DOCX 原始结构与有效格式
// =========================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DirectRunProps {
    pub font_size_pt: Option<f64>,
    pub ascii_font: Option<String>,
    pub east_asian_font: Option<String>,
    pub hansi_font: Option<String>,
    pub cs_font: Option<String>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub highlight: Option<String>,
    pub color: Option<String>,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleDefinition {
    pub style_id: String,
    pub name: Option<String>,
    pub based_on: Option<String>,
    pub next_paragraph_style: Option<String>,
    pub paragraph_props: DirectParagraphProps,
    pub run_props: DirectRunProps,
    pub is_default: bool,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FormatSources {
    /// key: property name, value: source description
    /// example: "font_size_pt" -> "run_direct"
    pub map: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectiveRunFormat {
    pub font_size_pt: Option<f64>,
    pub ascii_font: Option<String>,
    pub east_asian_font: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub highlight: Option<String>,
    pub color: Option<String>,
    pub sources: FormatSources,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectiveParagraphFormat {
    pub alignment: Option<Alignment>,
    pub line_spacing: Option<LineSpacing>,
    pub first_line_indent: Option<Indent>,
    pub left_indent: Option<Indent>,
    pub hanging_indent: Option<Indent>,
    pub outline_level: Option<u8>,
    pub style_id: Option<String>,
    pub sources: FormatSources,
}

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
    pub direct: DirectRunProps,
    pub character_style_id: Option<String>,
    pub effective: Option<EffectiveRunFormat>,
    pub inline_objects: Vec<InlineObject>,
    pub text_class_summary: TextClassSummary,
}

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
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paragraph {
    pub global_index: usize,
    pub block_index: usize,
    pub section_index: usize,
    pub text: String,
    pub runs: Vec<Run>,
    pub direct: DirectParagraphProps,
    pub style_id: Option<String>,
    pub effective: Option<EffectiveParagraphFormat>,
    pub role: ParagraphRole,
    pub role_tags: Vec<RoleTag>,
    pub role_confidence: f32,
    pub page_hint: Option<u32>,
    pub in_table: bool,
    pub in_header_footer: bool,
    pub in_footnote_endnote: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableCell {
    pub row_index: usize,
    pub col_index: usize,
    pub paragraph_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableRow {
    pub index: usize,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableBlock {
    pub block_index: usize,
    pub row_count: usize,
    pub col_count: usize,
    pub rows: Vec<TableRow>,
    pub paragraph_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Block {
    Paragraph(Paragraph),
    Table(TableBlock),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SectionProperties {
    pub index: usize,
    pub page_size: Option<PageSize>,
    pub margins: Option<Margins>,
    pub page_number_start: Option<u32>,
    pub restart_page_numbering: Option<bool>,
    pub title_page: Option<bool>,
    pub header_references: Vec<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CitationOccurrence {
    pub paragraph_index: usize,
    pub run_index: Option<usize>,
    pub raw: String,
    pub numbers: Vec<u32>,
    pub page_hint: Option<u32>,
}

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

// =========================
// 解析后的文档
// =========================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedDocument {
    pub path: PathBuf,
    pub file_name: String,
    pub sections: Vec<SectionProperties>,
    pub blocks: Vec<Block>,
    pub paragraphs: Vec<Paragraph>,
    pub styles: HashMap<String, StyleDefinition>,
    pub doc_defaults: Option<StyleDefinition>,
    pub captions: Vec<CaptionInfo>,
    pub citations: Vec<CitationOccurrence>,
    pub reference_entries: Vec<ReferenceEntry>,
    pub page_break_count: u32,
    pub parse_warnings: Vec<String>,
}

// =========================
// 规则 DSL
// =========================

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDefaults {
    pub tolerance: Tolerance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunApply {
    AllRuns,
    NonEmptyRuns,
    PrimaryRun,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineSpacingExpectation {
    pub rule: String,
    pub factor: Option<f64>,
    pub min_points: Option<f64>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionNumberingRule {
    #[serde(default = "default_one")]
    pub start: u32,
    #[serde(default = "default_true")]
    pub continuous: bool,
    #[serde(default)]
    pub allow_chapter_prefix: bool,
}

fn default_true() -> bool { true }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileOverride {
    #[serde(default)]
    pub overrides: Vec<RuleOverride>,
    #[serde(default)]
    pub disabled: Vec<String>,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontPolicy {
    #[serde(default)]
    pub allowed_fonts: Vec<String>,
    #[serde(default)]
    pub blocked_decorative_fonts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConfig {
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub defaults: RuleDefaults,
    #[serde(default)]
    pub font_size_aliases: HashMap<String, f64>,
    pub target_inference: TargetInferenceConfig,
    pub font_policy: FontPolicy,
    #[serde(default)]
    pub profiles: HashMap<String, ProfileOverride>,
    pub rules: Vec<Rule>,
}

// =========================
// 审核结果
// =========================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub section_index: Option<usize>,
    pub block_index: Option<usize>,
    pub paragraph_index: Option<usize>,
    pub run_index: Option<usize>,
    pub page_hint: Option<u32>,
    pub object_ref: Option<String>,
}

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
    pub confidence: f32,
    pub requires_manual_review: bool,
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
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Passed,
    NeedsRevision,
    ReadFailed,
    Skipped,
}

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

pub struct AuditContext<'a> {
    pub document: &'a ParsedDocument,
    pub config: &'a RuleConfig,
    pub rule: &'a Rule,
}

pub trait Checker: Send + Sync {
    fn rule_id(&self) -> &str;

    fn check_document(&self, ctx: &AuditContext<'_>) -> Vec<Violation> {
        let _ = ctx;
        Vec::new()
    }

    fn check_paragraph(&self, ctx: &AuditContext<'_>, paragraph: &'a Paragraph) -> Vec<Violation>
    where
        'a: 'ctx,
    {
        let _ = (ctx, paragraph);
        Vec::new()
    }
}