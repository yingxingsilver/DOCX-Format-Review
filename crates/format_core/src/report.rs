//! report —— 面向非技术人员的中文审核报告生成（P6）。
//!
//! 提供两种离线、单文件、无网络依赖的报告输出：
//! - [`generate_html_report`]：内联 daisyUI/Tailwind CSS + 原生 `<details>`/极简 JS，
//!   首页摘要、文件列表可展开、违规高亮、严重级别筛选。
//! - [`generate_csv_report`]：UTF-8 BOM + CRLF，Excel 直接打开不乱码。
//!
//! 约定：
//! - 所有用户可见文案中文、通俗；不出现 XML 节点名等技术词，
//!   技术细节只放在“技术详情”折叠区（[`TECH_DETAILS_LABEL`]）。
//! - 本模块不依赖 Tauri，不做任何网络请求，CSS 全部内联。
//! - annotated_docx（原文件加高亮副本）为后续阶段功能，未在本阶段引入。

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use crate::dsl::{AuditReport, FileStatus, Severity, Violation};
use crate::error::{CoreError, CoreResult};

/// 原文摘录最大展示字数（超出截断并加省略号）。
pub const EXCERPT_MAX_CHARS: usize = 80;

// =========================
// 公共入口
// =========================

/// 生成单文件 HTML 报告（内联 CSS，离线可用，中文界面）。
///
/// 失败时返回 [`CoreError::Io`]，`context` 为中文描述。
pub fn generate_html_report(report: &AuditReport, output_path: &Path) -> CoreResult<()> {
    let html = render_html(report);
    write_text(output_path, &html).map_err(|source| CoreError::Io {
        context: format!("写出HTML报告：{}", output_path.display()),
        source,
    })
}

/// 生成 CSV 报告。列顺序见 [`CSV_HEADERS`]。
pub fn generate_csv_report(report: &AuditReport, output_path: &Path) -> CoreResult<()> {
    let csv = render_csv(report);
    write_bytes(output_path, csv.as_bytes()).map_err(|source| CoreError::Io {
        context: format!("写出CSV报告：{}", output_path.display()),
        source,
    })
}

fn write_text(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(content.as_bytes())
}

fn write_bytes(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(content)
}

// =========================
// 术语通俗化（面向非技术人员）
// =========================

/// 把常见 OOXML 原始单位表达翻译成通俗中文（审查要求第 7 条）。
///
/// 仅做“能识别就翻译”的温和替换；无法识别的内容原样保留
/// （Violation 的 expected/actual 由各 Checker 负责产出中文）。
/// 注意：翻译后的文案保证不含 w: 前缀等技术标记。
pub fn plain_term(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    // 纯字号表达（"12pt"、"12 磅"、"12"）→ 通俗别名；带其他内容的句子原样清洗。
    // 注意先排除 "key=value" 属性串（如 lineRule=exact line=400），避免把行距值误读成字号。
    if !t.contains('=') {
        if let Some(pts) = parse_points(t) {
            return match font_size_alias_cn(pts) {
                Some(alias) => format!("{alias}（{}磅）", round_pt(pts)),
                None => format!("{} 磅", round_pt(pts)),
            };
        }
    }
    // 行距固定值：如 "lineRule=exact line=400" → "固定值 20 磅"（line 单位 1/20 磅）。
    // 注意：OOXML 的 lineRule 取值本身是枚举词（auto / exact / atLeast），
    // 数字形式（"1"/"4"）只出现在 w:spacing 属性里；这里两种写法都接受。
    if let Some(rule) = extract_key_value(t, "linerule") {
        // OOXML ST_LineSpacingRule 的合法取值只有 auto / exact / atLeast，
        // 大小写与首尾空白不敏感（属性串可能来自不同序列化实现）。
        let rl = rule.trim().to_ascii_lowercase();
        if rl == "exact" || rl == "atleast" || rl == "4" || rl == "1" {
            if let Some(p) = extract_key_number(t, "line") {
                return format!("固定值 {} 磅", round_pt(p / 20.0));
            }
        }
    }
    // twips 显式出现时翻译为毫米
    let lower = t.to_ascii_lowercase();
    if lower.contains("twip") {
        if let Some(v) = first_number(t) {
            return format!("约 {:.1} 毫米", v * 25.4 / 1440.0);
        }
    }
    // 兜底：去掉 OOXML 属性名残留（w:xxx / xxx=），保留数值与中文
    strip_tech_marks(t)
}

/// 中文字号别名表（pt → 别名）。
pub fn font_size_alias_cn(pt: f64) -> Option<&'static str> {
    let cases: [(f64, &str); 10] = [
        (42.0, "初号"),
        (36.0, "小初"),
        (26.0, "一号"),
        (24.0, "小一"),
        (22.0, "二号"),
        (18.0, "小二"),
        (16.0, "三号"),
        (15.0, "小三"),
        (14.0, "四号"),
        (12.0, "小四"),
    ];
    cases
        .iter()
        .find(|(v, _)| (v - pt).abs() < 0.01)
        .map(|(_, name)| *name)
}

/// 从字符串中提取第一个数值（支持小数）。
fn first_number(s: &str) -> Option<f64> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut seen_dot = false;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || (bytes[i] == '.' && !seen_dot)) {
                if bytes[i] == '.' {
                    seen_dot = true;
                }
                i += 1;
            }
            return bytes[start..i].iter().collect::<String>().parse().ok();
        }
        i += 1;
    }
    None
}

/// 解析 “12pt / 12 磅 / 12” 这类纯数值表达中的磅值。
///
/// 仅当整体是“数字 + 可选单位”时才返回，避免把句子误判成字号。
fn parse_points(s: &str) -> Option<f64> {
    let compact = s.to_ascii_lowercase().replace(' ', "");
    let core = compact.trim_end_matches("pt").trim_end_matches("磅");
    if core.is_empty() {
        // 只有单位没有数字（如 "pt"）：不是字号表达。
        return None;
    }
    if core == compact {
        // 没有任何单位时，也必须整体是数字（如 "12"）。
        if !compact.chars().all(|c| c.is_ascii_digit() || c == '.') || compact.is_empty() {
            return None;
        }
        return compact.parse::<f64>().ok();
    }
    core.parse::<f64>().ok()
}

/// 提取 `key=value` 形式的字符串值（大小写不敏感、忽略空格）。
///
/// 值的截断规则（关键修复点）：由于压缩空白会把相邻属性粘连
/// （"lineRule=exact line=400" → "linerule=exactline=400"），
/// 旧实现“先整体压掉空白再子串查找”会把下一个键名吞进当前值
/// （得到 "exactline=400"），导致查 "line" 永远失配、行距翻译
/// 落入兜底清洗只剩数字。现在改为：
///   - gap 表记录压缩前每个字符之前原本是否有空白；
///   - 候选键必须出现在原串的词边界上（串首或空白之后）；
///   - 值扫描遇到 `;` 停止；遇到形如 `xxx=` 的 token 时，仅当该
///     token 在原串中确实紧跟空白才视为“下一个键”并截断，否则
///     把它当作值的一部分继续扫描。
/// 例："lineRule=exact line=400" 中查 "linerule" 得到 "exact"；
///     查 "line" 得到 "400"。
fn extract_key_value(s: &str, key: &str) -> Option<String> {
    // gap[i] == true 表示压缩串第 i 个 char 之前，原串中存在空白。
    let mut compact = String::with_capacity(s.len());
    let mut gap: Vec<bool> = Vec::with_capacity(s.len());
    {
        let mut pending = false;
        for c in s.chars() {
            if c.is_whitespace() {
                pending = true;
                continue;
            }
            for lc in c.to_ascii_lowercase().to_lowercase() {
                compact.push(lc);
                gap.push(pending);
            }
            pending = false;
        }
    }
    debug_assert_eq!(compact.chars().count(), gap.len());
    let chars: Vec<char> = compact.chars().collect();
    let n = chars.len();
    let mut found: Option<String> = None;
    let mut idx = 0usize;
    // 逐个考察 key 在压缩串中的所有出现位置；只接受位于“词边界”上的键。
    while let Some(rel) = compact[idx..].find(key) {
        let p = idx + rel; // 键起点（本串全 ASCII，字节数 == 字符数）
        let before_ok = p == 0 || {
            // 键前一字符不是字母/数字，或者键本身紧跟在原串空白之后。
            gap[p] || !(chars[p - 1].is_ascii_alphanumeric())
        };
        let after_ok = p + key.len() < n && chars[p + key.len()] == '=';
        if before_ok && after_ok {
            let start = p + key.len() + 1;
            let mut end = start;
            while end < n {
                let c = chars[end];
                if c == ';' {
                    break;
                }
                if c.is_ascii_alphabetic() {
                    let mut j = end;
                    while j < n && chars[j].is_ascii_alphanumeric() {
                        j += 1;
                    }
                    // 只有“原串空白之后开始的 xxx=”才算下一个键的边界；
                    // 粘连进值里的同名片段（如 exact|line=）不算。
                    if j < n && chars[j] == '=' && (end == start || gap[end]) {
                        break;
                    }
                    end = j.max(end + 1);
                    continue;
                }
                end += 1;
            }
            let val: String = chars[start..end].iter().collect();
            if !val.is_empty() {
                found = Some(val); // 多处命中取最后一处
            }
        }
        idx = p + key.len();
    }
    found
}

/// 提取 `key=数值` 中的数值（取第一个可解析的数值段）。
fn extract_key_number(s: &str, key: &str) -> Option<f64> {
    let v = extract_key_value(s, key)?;
    let mut end = 0usize;
    let bytes = v.as_bytes();
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    v[..end].parse::<f64>().ok().filter(|n| n.is_finite())
}

/// 去除 OOXML 属性名等技术标记，保留数值与中文（用于兜底清洗）。
///
/// 策略：删除全部拉丁字母、`:`、`=`，把 `-` 换成顿号，压缩多余空白。
/// 例："宋体 SimSun sz=24" → "宋体 24"。
fn strip_tech_marks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        match c {
            '-' => out.push('、'),
            ':' | '=' => {}
            c if c.is_ascii_alphabetic() => {}
            c if c.is_whitespace() => {
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            }
            c => {
                out.push(c);
                prev_space = false;
            }
        }
    }
    out.trim().to_string()
}

/// 磅值展示：整数去掉小数点（20.0 → "20"）。
fn round_pt(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.1}", v)
    }
}

// =========================
// 定位信息人话化
// =========================

fn location_human(v: &Violation) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(pg) = v.location.page_hint {
        parts.push(format!("大约第 {pg} 页"));
    }
    if let Some(pi) = v.location.paragraph_index {
        parts.push(format!("第 {} 段", pi + 1));
    }
    if parts.is_empty() {
        parts.push("全文范围".to_string());
    }
    parts.join(" · ")
}

/// 截断摘录到至多 [`EXCERPT_MAX_CHARS`] 个字符（按 Unicode 字符计）。
pub fn truncate_excerpt(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= EXCERPT_MAX_CHARS {
        return text.to_string();
    }
    let mut s: String = chars[..EXCERPT_MAX_CHARS].iter().collect();
    s.push('…');
    s
}

// =========================
// HTML 转义（手写，零依赖）
// =========================

/// HTML 文本转义（同时可用于属性值的双引号场景）。
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

// =========================
// CSV
// =========================

/// CSV 列头（与需求一致）。
pub const CSV_HEADERS: [&str; 9] = [
    "文件名",
    "页码估算",
    "段落序号",
    "严重级别",
    "规则ID",
    "问题说明",
    "期望值",
    "实际值",
    "原文摘录",
];

/// 单个 CSV 字段转义（RFC4180），结果写入 `out`，避免逐字段临时分配。
fn write_csv_field(out: &mut String, field: &str) {
    let need_quote = field.contains(',')
        || field.contains('"')
        || field.contains('\n')
        || field.contains('\r');
    if !need_quote {
        out.push_str(field);
        return;
    }
    out.push('"');
    for c in field.chars() {
        if c == '"' {
            out.push_str("\"\"");
        } else {
            out.push(c);
        }
    }
    out.push('"');
}

/// 将一行字段以 CRLF 结尾写入 `out`。
fn write_csv_row(out: &mut String, fields: &[&str]) {
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_csv_field(out, f);
    }
    out.push_str("\r\n");
}

fn csv_fields(v: &Violation) -> [String; 9] {
    let file_name = file_display_name(&v.file_path);
    let page = v
        .location
        .page_hint
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let para = v
        .location
        .paragraph_index
        .map(|i| (i + 1).to_string())
        .unwrap_or_else(|| "-".to_string());
    let excerpt = v
        .excerpt
        .as_deref()
        .map(truncate_excerpt)
        .unwrap_or_default();
    [
        file_name,
        page,
        para,
        v.severity.label_cn().to_string(),
        v.rule_id.clone(),
        v.message.clone(),
        v.expected.clone().unwrap_or_default(),
        v.actual.clone().unwrap_or_default(),
        excerpt,
    ]
}

/// 将 `[String; N]` 字段数组转为 `&[&str]` 借用视图，供 write_csv_row 零转义写入。
fn as_str_refs<const N: usize>(fields: &[String; N]) -> [&str; N] {
    std::array::from_fn(|i| fields[i].as_str())
}

fn file_display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string())
}

/// 渲染完整 CSV 文本（含 UTF-8 BOM，Excel 友好；CRLF 行尾）。
pub fn render_csv(report: &AuditReport) -> String {
    let mut out = String::with_capacity(64 + report.files.len() * 256);
    out.push('\u{feff}');
    // 表头为固定短字段，无需转义，直接内联。
    out.push_str("文件名,页码估算,段落序号,严重级别,规则ID,问题说明,期望值,实际值,原文摘录\r\n");
    for file in &report.files {
        if file.status == FileStatus::ReadFailed {
            // 读取失败也占一行，方便用户在表格里看到哪些文件没检查成功。
            let msg = file
                .parse_error
                .as_deref()
                .unwrap_or("文件无法读取。");
            let fields = [
                file.file_name.as_str(),
                "-",
                "-",
                "错误",
                "file-unreadable",
                msg,
                "",
                "",
                "",
            ];
            write_csv_row(&mut out, &fields);
        }
        for v in &file.violations {
            let fields = csv_fields(v);
            write_csv_row(&mut out, &as_str_refs(&fields));
        }
    }
    out
}

// =========================
// HTML 渲染
// =========================

/// daisyUI light 主题变量（构建期由 node_modules/daisyui/dist/themes.css 裁剪而来，
/// 运行时零网络依赖；更新 daisyUI 版本时请同步刷新此常量）。
const DAISYUI_THEME_VARS_CSS: &str = r#":root{color-scheme:light;--in:72.06% 0.191 231.6;--su:64.8% 0.150 160;--wa:84.71% 0.199 83.87;--er:71.76% 0.221 22.18;--pc:89.824% 0.06192 275.75;--ac:15.352% 0.0368 183.61;--inc:0% 0 0;--suc:0% 0 0;--wac:0% 0 0;--erc:0% 0 0;--rounded-box:1rem;--rounded-btn:0.5rem;--rounded-badge:1.9rem;--animation-btn:0.25s;--animation-input:.2s;--btn-focus-scale:0.95;--border-btn:1px;--tab-border:1px;--tab-radius:0.5rem;--p:49.124% 0.03096 275.75;--s:69.715% 0.03297 266.71;--sc:98.185% 0.0108 266.73;--a:76.76% 0.184 183.61;--b1:100% 0 0;--b2:97% 0 0;--b3:94% 0 0;--bc:0% 0 0;--fc:100% 0 0;--nf:14.904% 0.008374 283.09;--nc:71.391% 0.014746 284.05;--b1h:97% 0 0;--b2h:94% 0 0;--b3h:90% 0 0;--bah:94% 0 0;--bh:94% 0 0}"#;

/// daisyUI 组件子集样式（card/badge/alert/btn/stats/table/collapse 关键规则，
/// 由 node_modules/daisyui/dist/styled.css 裁剪，类名与 daisyUI 保持一致）。
const DAISYUI_COMPONENTS_CSS: &str = r#".card{display:flex;flex-direction:column;border-radius:var(--rounded-box,1rem);background-color:var(--fallback-b1,#fff);color:inherit}.card-body{display:flex;flex-direction:column;gap:.5rem;padding:1.5rem;flex:auto}.card-title{display:flex;align-items:center;font-weight:600;font-size:1.1rem}.badge{display:inline-flex;align-items:center;justify-content:center;height:1.25rem;font-size:.8rem;width:fit-content;padding-left:.567rem;padding-right:.567rem;border-width:1px;border-radius:var(--rounded-badge,1.9rem)}.badge-error{border-color:var(--fallback-er,oklch(var(--er)));background-color:var(--fallback-er,oklch(var(--er)));color:var(--fallback-erc,oklch(var(--erc)))}.badge-warning{border-color:var(--fallback-wa,oklch(var(--wa)));background-color:var(--fallback-wa,oklch(var(--wa)));color:var(--fallback-wac,oklch(var(--wac)))}.badge-info{border-color:var(--fallback-in,oklch(var(--in)));background-color:var(--fallback-in,oklch(var(--in)));color:var(--fallback-inc,oklch(var(--inc)))}.badge-success{border-color:var(--fallback-su,oklch(var(--su)));background-color:var(--fallback-su,oklch(var(--su)));color:var(--fallback-suc,oklch(var(--suc)))}.alert{display:flex;align-items:center;gap:.5rem;border-radius:var(--rounded-box,1rem);padding:1rem;background-color:var(--fallback-b2,#fff)}.alert-success{border-color:var(--fallback-su,oklch(var(--su)));background-color:var(--fallback-su,oklch(var(--su)));color:var(--fallback-suc,oklch(var(--suc)))}.alert-warning{border-color:var(--fallback-wa,oklch(var(--wa)));background-color:var(--fallback-wa,oklch(var(--wa)));color:var(--fallback-wac,oklch(var(--wac)))}.alert-error{border-color:var(--fallback-er,oklch(var(--er)));background-color:var(--fallback-er,oklch(var(--er)));color:var(--fallback-erc,oklch(var(--erc)))}.btn{display:inline-flex;flex-wrap:nowrap;align-items:center;justify-content:center;height:3rem;padding-left:1rem;padding-right:1rem;font-size:.875rem;font-weight:600;border-radius:var(--rounded-btn,.5rem);border-width:var(--border-btn,1px);cursor:pointer;color:var(--fallback-bc,oklch(var(--bc)));background-color:var(--fallback-b1,#fff);min-height:2rem}.btn-outline.btn-error{border-color:var(--fallback-er,oklch(var(--er)));color:var(--fallback-er,oklch(var(--er)))}.btn-outline.btn-error.btn-active,.btn-outline.btn-error[aria-pressed="true"]{background-color:var(--fallback-er,oklch(var(--er)));color:var(--fallback-erc,oklch(var(--erc)))}.btn-outline.btn-warning{border-color:var(--fallback-wa,oklch(var(--wa)));color:var(--fallback-wa,oklch(var(--wa)))}.btn-outline.btn-warning.btn-active,.btn-outline.btn-warning[aria-pressed="true"]{background-color:var(--fallback-wa,oklch(var(--wa)));color:var(--fallback-wac,oklch(var(--wac)))}.btn-outline.btn-info{border-color:var(--fallback-in,oklch(var(--in)));color:var(--fallback-in,oklch(var(--in)))}.btn-outline.btn-info.btn-active,.btn-outline.btn-info[aria-pressed="true"]{background-color:var(--fallback-in,oklch(var(--in)));color:var(--fallback-inc,oklch(var(--inc)))}.btn-outline.btn-active,.btn-outline[aria-pressed="true"]{background-color:var(--fallback-bc,oklch(var(--bc)));color:var(--fallback-b1,#fff)}.stats{display:inline-grid;overflow:hidden;border-radius:var(--rounded-box,1rem);grid-flow:column;background-color:#fff}.stat{display:inline-grid;width:100%;padding:1rem 1.5rem;gap:.25rem;border-width:1px}.stat-title{opacity:.6;font-size:.75rem}.stat-value{font-size:1.6rem;font-weight:600;line-height:1.2}.stat-desc{font-size:.75rem;opacity:.75}.table{width:100%;text-align:left;border-radius:var(--rounded-box,1rem);border-collapse:separate;border-spacing:0;font-size:.9rem}.table th,.table td{padding:.6rem .8rem;border-bottom:1px solid #e5e7eb}.table tr:last-child td{border-bottom:0}.collapse{visibility:collapse;border-radius:var(--rounded-box,1rem)}.collapse-title{visibility:visible}.collapse-content{visibility:hidden;overflow:hidden}.collapse-open,.collapse:not(.collapse-close){visibility:visible}.collapse :checked~.collapse-content,[open]>.collapse-content{visibility:visible}details.collapse summary{list-style:none;cursor:pointer}details.collapse summary::-webkit-details-marker{display:none}details.collapse[open] .collapse-content{visibility:visible;padding:0 1rem 1rem}details.collapse .collapse-title{padding:1rem;font-weight:600}"#;

/// 报告专属布局与高亮样式（hl-error/hl-warning/hl-info 为需求第 5 条的高亮 class）。
const REPORT_LAYOUT_CSS: &str = r#"body{margin:0;background:#f3f4f6;color:#111827;font-family:system-ui,"Microsoft YaHei","PingFang SC",sans-serif;line-height:1.6}.wrap{max-width:1080px;margin:0 auto;padding:1.5rem 1rem}h1{font-size:1.5rem;margin:0 0 .25rem}.muted{color:#6b7280;font-size:.85rem}.stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));width:100%}.filters{display:flex;gap:.5rem;flex-wrap:wrap;margin:1rem 0}.filters .btn{height:2.2rem;font-size:.8rem}.file{margin-bottom:.75rem;border:1px solid #e5e7eb}.violation{border:1px solid #e5e7eb;border-radius:.5rem;padding:.6rem .8rem;margin:.5rem 0;background:#fff}.violation[data-sev="error"]{border-left:.4rem solid oklch(var(--er))}.violation[data-sev="warning"]{border-left:.4rem solid oklch(var(--wa))}.violation[data-sev="info"]{border-left:.4rem solid oklch(var(--in))}.excerpt{margin-top:.4rem;padding:.3rem .5rem;background:#f9fafb;border-radius:.3rem;font-size:.9rem;word-break:break-all}.hl-error{background:rgba(239,68,68,.35);padding:0 .15rem;border-radius:.2rem}.hl-warning{background:rgba(245,158,11,.4);padding:0 .15rem;border-radius:.2rem}.hl-info{background:rgba(59,130,246,.3);padding:0 .15rem;border-radius:.2rem}.kv{display:grid;grid-template-columns:5em 1fr;gap:.1rem .5rem;font-size:.9rem;margin-top:.3rem}.kv dt{color:#6b7280}.tech{font-size:.8rem;color:#6b7280;margin-top:.4rem}.hidden{display:none!important}.topbar{position:sticky;top:0;background:#f3f4f6ee;z-index:10;padding:.5rem 0}.empty{padding:1rem;text-align:center;color:#6b7280}"#;

const FILTER_SCRIPT: &str = r#"(function(){
  var buttons=document.querySelectorAll('[data-filter]');
  function apply(kind){
    document.querySelectorAll('.violation').forEach(function(el){
      el.classList.toggle('hidden', kind!=='all' && el.getAttribute('data-sev')!==kind);
    });
    document.querySelectorAll('.filebox').forEach(function(box){
      if(kind==='all'){box.classList.remove('hidden');return;}
      var any=box.querySelector('.violation[data-sev="'+kind+'"]:not(.hidden)');
      box.classList.toggle('hidden', !any);
    });
    buttons.forEach(function(b){b.setAttribute('aria-pressed', b.getAttribute('data-filter')===kind?'true':'false');});
  }
  buttons.forEach(function(b){b.addEventListener('click',function(){apply(b.getAttribute('data-filter'));});});
})();"#;

fn severity_css_class(s: Severity) -> &'static str {
    match s {
        Severity::Error => "hl-error",
        Severity::Warning => "hl-warning",
        Severity::Info => "hl-info",
    }
}

fn severity_badge(s: Severity) -> &'static str {
    match s {
        Severity::Error => "badge badge-error",
        Severity::Warning => "badge badge-warning",
        Severity::Info => "badge badge-info",
    }
}

fn status_label_cn(s: &FileStatus) -> &'static str {
    match s {
        FileStatus::Passed => "通过",
        FileStatus::NeedsRevision => "需修改",
        FileStatus::ReadFailed => "读取失败",
        FileStatus::Skipped => "未检查（已取消）",
    }
}

fn status_badge(s: &FileStatus) -> &'static str {
    match s {
        FileStatus::Passed => "badge badge-success",
        FileStatus::NeedsRevision => "badge badge-warning",
        FileStatus::ReadFailed => "badge badge-error",
        FileStatus::Skipped => "badge badge-info",
    }
}

/// 错误/警告数量 Top 规则统计：(rule_id, 错误数, 警告数)。
pub fn top_rules_by_severity(report: &AuditReport, limit: usize) -> Vec<(String, usize, usize)> {
    let mut map: std::collections::HashMap<String, (usize, usize)> = std::collections::HashMap::new();
    for file in &report.files {
        for v in &file.violations {
            let e = map.entry(v.rule_id.clone()).or_insert((0, 0));
            match v.severity {
                Severity::Error => e.0 += 1,
                Severity::Warning => e.1 += 1,
                Severity::Info => {}
            }
        }
    }
    let mut rows: Vec<(String, usize, usize)> = map
        .into_iter()
        .map(|(id, (e, w))| (id, e, w))
        .collect();
    // 排序：错误数降序 → 警告数降序 → 规则 ID 升序（保证结果确定性）。
    rows.sort_by(|a, b| {
        (b.1, b.2)
            .cmp(&(a.1, a.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.truncate(limit);
    rows
}

/// 一条违规对应的中文规则说明：优先用 message 本身（Checker 已保证中文），
/// 这里保留函数以便未来接入规则名字典。
fn rule_explanation(v: &Violation) -> String {
    v.message.clone()
}

fn render_violation(v: &Violation) -> String {
    use std::cell::OnceCell;
    let sev = v.severity;
    // 预分配容量，减少常见报告规模下的反复扩容拷贝。
    let mut s = String::with_capacity(512);
    // 技术详情里要用的字符串先惰性计算一次，避免多次 clone/unwrap_or_else 分配。
    let obj_ref_cell: OnceCell<String> = OnceCell::new();
    let obj_ref = || obj_ref_cell.get_or_init(|| v.location.object_ref.clone().unwrap_or_default());
    s.push_str(&format!(
        "<div class=\"violation\" data-sev=\"{}\">",
        match sev {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    ));
    s.push_str(&format!(
        "<span class=\"{}\">{}</span> <span class=\"muted\">检查项：{}</span>",
        severity_badge(sev),
        sev.label_cn(),
        escape_html(&v.rule_id)
    ));
    s.push_str(&format!(
        "<p style=\"margin:.3rem 0 0\">{}</p>",
        escape_html(&rule_explanation(v))
    ));
    s.push_str("<dl class=\"kv\">");
    s.push_str(&format!("<dt>位置</dt><dd>{}</dd>", escape_html(&location_human(v))));
    if let Some(exp) = &v.expected {
        // Checker 产出的 expected 已是中文（如“小四（12pt）”），这里只做属性名清洗，
        // 不做二次翻译，避免破坏原有措辞。
        s.push_str(&format!(
            "<dt>期望格式</dt><dd>{}</dd>",
            escape_html(&strip_tech_marks(exp))
        ));
    }
    if let Some(act) = &v.actual {
        s.push_str(&format!(
            "<dt>实际格式</dt><dd>{}</dd>",
            escape_html(&strip_tech_marks(act))
        ));
    }
    if let Some(sug) = &v.suggestion {
        s.push_str(&format!(
            "<dt>怎么改</dt><dd>{}</dd>",
            escape_html(sug)
        ));
    }
    s.push_str("</dl>");
    if let Some(ex) = &v.excerpt {
        let cut = truncate_excerpt(ex);
        s.push_str(&format!(
            "<div class=\"excerpt\">原文摘录：<mark class=\"{}\">{}</mark></div>",
            severity_css_class(sev),
            escape_html(&cut)
        ));
    }
    // 技术详情折叠区：允许出现内部标识（规则ID/置信度/对象引用），默认收起。
    let _ = write!(
        s,
        "<details class=\"tech\"><summary>技术详情（供维护人员排查，普通用户可忽略）</summary>规则编号：{}；置信度：{:.0}%；对象引用：{}</details>",
        escape_html(&v.rule_id),
        v.confidence * 100.0,
        escape_html(if obj_ref().is_empty() { "无" } else { obj_ref() })
    );
    s.push_str("</div>");
    s
}

fn render_file_card(file: &crate::dsl::FileAuditSummary) -> String {
    let mut s = String::with_capacity(1024 + file.violations.len() * 512);
    let total = file.error_count + file.warning_count + file.info_count;
    s.push_str(if total > 0 {
        "<details class=\"collapse card file\" open>"
    } else {
        "<details class=\"collapse card file\">"
    });
    s.push_str(&format!(
        "<summary class=\"collapse-title\">{} <span class=\"{}\">{}</span> \
         <span class=\"muted\">错误 {} · 警告 {} · 提示 {}</span></summary>",
        escape_html(&file.file_name),
        status_badge(&file.status),
        status_label_cn(&file.status),
        file.error_count,
        file.warning_count,
        file.info_count
    ));
    s.push_str("<div class=\"collapse-content filebox card-body\">");
    if let Some(err) = &file.parse_error {
        s.push_str(&format!(
            "<div class=\"alert alert-error\"><span>{}</span></div>",
            escape_html(err)
        ));
    }
    if file.violations.is_empty() && file.parse_error.is_none() {
        s.push_str("<p class=\"empty\">这个文件没有发现问题。</p>");
    }
    for v in &file.violations {
        s.push_str(&render_violation(v));
    }
    s.push_str("</div></details>");
    s
}

/// 渲染完整单文件 HTML（导出便于测试与前端预览复用）。
pub fn render_html(report: &AuditReport) -> String {
    let esc_title = "论文格式审核报告";
    // 预估总长度，避免反复 realloc；CSS 常量本身就有数 KB。
    let est = DAISYUI_THEME_VARS_CSS.len()
        + DAISYUI_COMPONENTS_CSS.len()
        + REPORT_LAYOUT_CSS.len()
        + FILTER_SCRIPT.len()
        + report.files.len() * 1024
        + 4096;
    let mut s = String::with_capacity(est);
    s.push_str("<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n");
    s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    s.push_str(&format!("<title>{esc_title}</title>\n<style>\n"));
    s.push_str(DAISYUI_THEME_VARS_CSS);
    s.push('\n');
    s.push_str(DAISYUI_COMPONENTS_CSS);
    s.push('\n');
    s.push_str(REPORT_LAYOUT_CSS);
    s.push_str("\n</style>\n</head>\n<body>\n<div class=\"wrap\">\n");

    // —— 顶部摘要 ——
    s.push_str(&format!("<h1>{esc_title}</h1>\n"));
    s.push_str(&format!(
        "<p class=\"muted\">生成时间：{} · 审核文件夹：{} · 规则文件：{}</p>\n",
        escape_html(&report.generated_at),
        escape_html(&report.input_dir.display().to_string()),
        escape_html(&report.rule_file.display().to_string())
    ));
    s.push_str("<div class=\"stats card\"><div class=\"stat\"><div class=\"stat-title\">共审核</div><div class=\"stat-value\">");
    s.push_str(&report.total_files.to_string());
    s.push_str("</div><div class=\"stat-desc\">篇论文</div></div>");
    s.push_str("<div class=\"stat\"><div class=\"stat-title\">通过</div><div class=\"stat-value\">");
    s.push_str(&report.passed_files.to_string());
    s.push_str("</div><div class=\"stat-desc\">没有问题或仅轻微提示</div></div>");
    s.push_str("<div class=\"stat\"><div class=\"stat-title\">需要修改</div><div class=\"stat-value\">");
    s.push_str(&report.failed_files.to_string());
    s.push_str("</div><div class=\"stat-desc\">存在格式问题</div></div>");
    s.push_str("<div class=\"stat\"><div class=\"stat-title\">读取失败</div><div class=\"stat-value\">");
    s.push_str(&report.read_failed_files.to_string());
    s.push_str("</div><div class=\"stat-desc\">文件损坏或打不开</div></div></div>\n");

    // 总体结论横幅
    if report.total_errors > 0 {
        s.push_str(&format!(
            "<div class=\"alert alert-error\" style=\"margin-top:1rem\"><span>本次共发现 <b>{}</b> 个必须修改的问题、<b>{}</b> 个建议修改的问题。</span></div>",
            report.total_errors, report.total_warnings
        ));
    } else if report.total_warnings > 0 {
        s.push_str(&format!(
            "<div class=\"alert alert-warning\" style=\"margin-top:1rem\"><span>没有必须修改的问题，但有 <b>{}</b> 个建议修改的地方。</span></div>",
            report.total_warnings
        ));
    } else {
        s.push_str("<div class=\"alert alert-success\" style=\"margin-top:1rem\"><span>太棒了，没有发现格式问题。</span></div>");
    }

    // —— Top 规则 ——
    let top = top_rules_by_severity(report, 5);
    if !top.is_empty() {
        s.push_str("<div class=\"card\" style=\"margin-top:1rem\"><div class=\"card-body\"><h2 class=\"card-title\">问题最多的检查项</h2><table class=\"table\"><thead><tr><th>检查项</th><th>错误数</th><th>警告数</th></tr></thead><tbody>");
        for (rid, e, w) in &top {
            s.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(rid),
                e,
                w
            ));
        }
        s.push_str("</tbody></table></div></div>\n");
    }

    // —— 筛选按钮 ——
    s.push_str(
        "<div class=\"filters\" role=\"toolbar\" aria-label=\"按严重程度筛选\">\
         <button type=\"button\" class=\"btn btn-outline\" data-filter=\"all\" aria-pressed=\"true\">全部显示</button>\
         <button type=\"button\" class=\"btn btn-outline btn-error\" data-filter=\"error\" aria-pressed=\"false\">只看错误</button>\
         <button type=\"button\" class=\"btn btn-outline btn-warning\" data-filter=\"warning\" aria-pressed=\"false\">只看警告</button>\
         <button type=\"button\" class=\"btn btn-outline btn-info\" data-filter=\"info\" aria-pressed=\"false\">只看提示</button>\
         </div>\n",
    );

    // —— 文件列表 ——
    s.push_str("<h2 style=\"font-size:1.1rem\">文件明细（点击文件名展开）</h2>\n");
    if report.files.is_empty() {
        s.push_str("<p class=\"empty\">没有找到任何论文文件。</p>");
    }
    for file in &report.files {
        s.push_str(&render_file_card(file));
        s.push('\n');
    }

    s.push_str(&format!("<script>{FILTER_SCRIPT}</script>\n</div>\n</body>\n</html>\n"));
    s
}

/// 生成一份演示用报告（供 `examples/gen_report.rs` 与 UI 预览使用）。
pub fn sample_demo_report() -> AuditReport {
    use crate::dsl::{FileAuditSummary, Location};
    let mk = |rule_id: &str,
              sev: Severity,
              file: &str,
              page: Option<u32>,
              para: Option<usize>,
              msg: &str,
              expected: Option<&str>,
              actual: Option<&str>,
              excerpt: Option<&str>| Violation {
        rule_id: rule_id.into(),
        severity: sev,
        file_path: std::path::PathBuf::from(file),
        location: Location {
            section_index: Some(0),
            block_index: None,
            paragraph_index: para,
            run_index: None,
            page_hint: page,
            object_ref: None,
        },
        message: msg.into(),
        expected: expected.map(str::to_string),
        actual: actual.map(str::to_string),
        excerpt: excerpt.map(str::to_string),
        suggestion: None,
        confidence: 0.9,
        requires_manual_review: false,
    };
    let a = FileAuditSummary {
        file_path: std::path::PathBuf::from("论文甲.docx"),
        file_name: "论文甲.docx".into(),
        status: FileStatus::NeedsRevision,
        error_count: 2,
        warning_count: 1,
        info_count: 0,
        violations: vec![
            mk("body-font-size", Severity::Error, "论文甲.docx", Some(4), Some(11),
               "正文字号不符合要求：应为小四（12磅），当前偏小。",
               Some("12pt"), Some("10.5pt"),
               Some("本文围绕深度学习图像识别展开研究，首先介绍了相关背景与国内外研究现状，然后……")),
            mk("line-spacing", Severity::Error, "论文甲.docx", Some(4), Some(12),
               "正文行距应为固定值 20 磅。",
               Some("lineRule=exact line=400"), Some("lineRule=auto line=360"),
               Some("实验结果表明该方法在准确率上有明显提升。")),
            mk("heading-level1", Severity::Warning, "论文甲.docx", Some(2), Some(3),
               "一级标题应使用黑体三号并居中。",
               Some("黑体、三号（16磅）、居中"), Some("宋体、左对齐"),
               Some("绪论")),
        ],
        parse_error: None,
    };
    let b = FileAuditSummary {
        file_path: std::path::PathBuf::from("论文乙.docx"),
        file_name: "论文乙.docx".into(),
        status: FileStatus::Passed,
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        violations: vec![],
        parse_error: None,
    };
    let c = FileAuditSummary {
        file_path: std::path::PathBuf::from("损坏文件.docx"),
        file_name: "损坏文件.docx".into(),
        status: FileStatus::ReadFailed,
        error_count: 0,
        warning_count: 0,
        info_count: 0,
        violations: vec![],
        parse_error: Some("无法读取该 Word 文档，文件可能已损坏或被加密。".into()),
    };
    AuditReport {
        generated_at: "2026-10-11 12:00:00".into(),
        input_dir: std::path::PathBuf::from("论文"),
        rule_file: std::path::PathBuf::from("rules/default.json"),
        rule_profile: None,
        total_files: 3,
        passed_files: 2,
        failed_files: 1,
        read_failed_files: 1,
        total_errors: 2,
        total_warnings: 1,
        total_infos: 0,
        files: vec![a, b, c],
    }
}

// =========================
// 调试辅助（供外部测试/排查使用）
// =========================

/// 调试用：提取串中 `linerule=` 的值。
#[doc(hidden)]
pub fn debug_kv(s: &str) -> Option<String> {
    extract_key_value(s, "linerule")
}

/// 调试用：提取串中 `line=` 的数值。
#[doc(hidden)]
pub fn debug_line(s: &str) -> Option<f64> {
    extract_key_number(s, "line")
}

// =========================
// 单元测试
// =========================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{FileAuditSummary, Location};
    use std::path::PathBuf;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fmtcore-report-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mk_violation(rule_id: &str, sev: Severity, msg: &str, excerpt: &str) -> Violation {
        Violation {
            rule_id: rule_id.into(),
            severity: sev,
            file_path: PathBuf::from("/论文/示例甲.docx"),
            location: Location {
                section_index: Some(0),
                block_index: Some(3),
                paragraph_index: Some(11),
                run_index: None,
                page_hint: Some(4),
                object_ref: None,
            },
            message: msg.into(),
            expected: Some("小四（12pt）".into()),
            actual: Some("五号（10.5pt）".into()),
            excerpt: Some(excerpt.into()),
            suggestion: Some("选中这段文字，把字号改成“小四”。".into()),
            confidence: 0.95,
            requires_manual_review: false,
        }
    }

    /// 需求测试 1：构造包含 3 条违规的 AuditReport。
    fn sample_report() -> AuditReport {
        let v1 = mk_violation("body-font-size", Severity::Error, "正文字号不符合学校要求：应为小四（12磅）。", "本文围绕深度学习展开研究");
        let v2 = mk_violation("line-spacing", Severity::Warning, "行距建议使用固定值 20 磅。", "实验结果表明该方法有效，");
        let v3 = mk_violation("figure-caption", Severity::Info, "图 1-2 的编号与上一章不连续，请人工确认。", "如图 1-2 所示，模型结构分为三层");
        let file = FileAuditSummary {
            file_path: PathBuf::from("/论文/示例甲.docx"),
            file_name: "示例甲.docx".into(),
            status: FileStatus::NeedsRevision,
            error_count: 1,
            warning_count: 1,
            info_count: 1,
            violations: vec![v1, v2, v3],
            parse_error: None,
        };
        AuditReport {
            generated_at: "2026-10-11 12:00:00".into(),
            input_dir: PathBuf::from("/论文"),
            rule_file: PathBuf::from("rules/default.json"),
            rule_profile: None,
            total_files: 1,
            passed_files: 0,
            failed_files: 1,
            read_failed_files: 0,
            total_errors: 1,
            total_warnings: 1,
            total_infos: 1,
            files: vec![file],
        }
    }

    #[test]
    fn module_exists() {
        // 模块级自检：占位入口仍由 lib.rs 统一验证；此处验证公共 API 形状存在。
        let _: fn(&AuditReport, &std::path::Path) -> CoreResult<()> = generate_html_report;
        let _: fn(&AuditReport, &std::path::Path) -> CoreResult<()> = generate_csv_report;
    }

    /// 需求测试 2：HTML 包含文件名、规则说明、高亮 class。
    #[test]
    fn html_contains_filename_rule_text_and_highlight_classes() {
        let html = render_html(&sample_report());
        assert!(html.contains("示例甲.docx"), "缺少文件名");
        assert!(html.contains("正文字号不符合学校要求"), "缺少规则说明");
        assert!(html.contains("hl-error"), "缺少错误高亮 class");
        assert!(html.contains("hl-warning"), "缺少警告高亮 class");
        assert!(html.contains("hl-info"), "缺少提示高亮 class");
        assert!(html.contains("只看错误"), "缺少筛选按钮");
        assert!(html.contains("只看警告"), "缺少筛选按钮");
        // XML 节点名等技术词不得外露；技术信息只出现在“技术详情”折叠区。
        assert!(!html.contains("<w:") && !html.contains("pPr"), "不应外露 XML 节点名");
        assert!(html.contains("技术详情"));
        assert!(html.contains("daisyui") || html.contains("--er:"), "缺少 daisyUI 内联主题变量");
        // 单文件离线：不得引用外部 http 资源
        assert!(!html.contains("http://") && !html.contains("https://"), "不应联网加载资源");
        // 页码估算与段落序号（人话化，1-based）
        assert!(html.contains("大约第 4 页"));
        assert!(html.contains("第 12 段"));
    }

    /// 需求测试 3：CSV 行数与列名正确。
    #[test]
    fn csv_has_expected_header_and_rows() {
        let csv = render_csv(&sample_report());
        let lines: Vec<&str> = csv.trim_end_matches("\r\n").split("\r\n").collect();
        // 1 表头 + 3 违规
        assert_eq!(lines.len(), 4, "CSV 行数不对: {lines:?}");
        assert_eq!(
            lines[0].trim_start_matches('\u{feff}'),
            "文件名,页码估算,段落序号,严重级别,规则ID,问题说明,期望值,实际值,原文摘录"
        );
        assert!(lines[1].starts_with("示例甲.docx,4,12,错误,body-font-size"));
        // 逗号/引号转义（RFC4180）：通过 write_csv_row 直接验证。
        let mut row = String::new();
        write_csv_row(&mut row, &["文件", "含\"引号\",和逗号"]);
        assert_eq!(row, "文件,\"含\"\"引号\"\",和逗号\"\r\n");
        // csv_fields 对违规的字段映射正确。
        let tricky = mk_violation("t", Severity::Info, "含\"引号\",和逗号", "a,b");
        let fields = csv_fields(&tricky);
        assert_eq!(fields[3], "提示");
        assert_eq!(fields[5], "含\"引号\",和逗号");
        let mut out2 = String::new();
        write_csv_row(&mut out2, &as_str_refs(&fields));
        assert!(out2.contains("\"含\"\"引号\"\",和逗号\""));
    }

    #[test]
    fn read_failed_file_gets_its_own_csv_row_and_chinese_reason() {
        let mut rep = sample_report();
        rep.files.push(FileAuditSummary {
            file_path: PathBuf::from("/论文/坏文件.docx"),
            file_name: "坏文件.docx".into(),
            status: FileStatus::ReadFailed,
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            violations: vec![],
            parse_error: Some("无法读取该 Word 文档，文件可能已损坏。".into()),
        });
        let csv = render_csv(&rep);
        assert!(csv.contains("坏文件.docx"));
        assert!(csv.contains("文件可能已损坏"));
        let html = render_html(&rep);
        assert!(html.contains("读取失败"));
    }

    #[test]
    fn excerpt_truncated_to_80_chars() {
        let long = "测".repeat(120);
        let cut = truncate_excerpt(&long);
        assert_eq!(cut.chars().count(), EXCERPT_MAX_CHARS + 1); // 80 + 省略号
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn html_escapes_injection() {
        let mut rep = sample_report();
        rep.files[0].violations[0].message = "<script>alert(1)</script>".into();
        let html = render_html(&rep);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn plain_term_translates_sizes_and_line_spacing() {
        assert_eq!(plain_term("12pt"), "小四（12磅）");
        assert_eq!(plain_term("12 磅"), "小四（12磅）");
        assert_eq!(plain_term("10.5pt"), "10.5 磅"); // 无别名时给出磅值
        assert_eq!(plain_term("lineRule=exact line=400"), "固定值 20 磅");
        // 句子类输入不被误翻译
        assert_eq!(plain_term("宋体加粗，居中"), "宋体加粗，居中");
        // 不含技术标记：清洗后不得残留拉丁字母属性名
        let out = plain_term("w:sz=24 w:szCs=24");
        assert!(!out.contains("w:") && !out.chars().any(|c| c.is_ascii_alphabetic()), "{out}");
    }

    #[test]
    fn top_rules_sorted_by_errors_then_warnings() {
        let mut rep = sample_report();
        let extra = mk_violation("body-font-size", Severity::Warning, "另一条字号警告", "摘录");
        rep.files[0].violations.push(extra);
        rep.files[0].warning_count += 1;
        let top = top_rules_by_severity(&rep, 5);
        assert_eq!(top[0].0, "body-font-size");
        assert_eq!((top[0].1, top[0].2), (1, 1));
    }

    #[test]
    fn generate_writes_files_to_disk() {
        let dir = tmp_dir("write");
        let rep = sample_report();
        let html_path = dir.join("报告.html");
        let csv_path = dir.join("报告.csv");
        generate_html_report(&rep, &html_path).unwrap();
        generate_csv_report(&rep, &csv_path).unwrap();
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("论文格式审核报告"));
        let csv_bytes = std::fs::read(&csv_path).unwrap();
        assert_eq!(&csv_bytes[..3], b"\xEF\xBB\xBF", "CSV 应带 UTF-8 BOM");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn io_error_context_is_chinese() {
        let bad = std::path::Path::new("/proc/definitely-not-writable-here/report.html");
        let e = generate_html_report(&sample_report(), bad)
            .expect_err("写入非法路径应失败");
        match e {
            CoreError::Io { ref context, .. } => {
                assert!(context.contains("写出HTML报告"), "{context}");
            }
            other => panic!("应为 Io 错误，得到 {other:?}"),
        }
        assert!(e.user_message().contains("文件"));
    }
}
