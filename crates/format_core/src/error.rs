//! error —— 核心引擎统一错误类型（P1）。
//!
//! 约定：
//! - `CoreError` 表达“整个文件/整次运行失败”的错误（无法解析、配置非法等）；
//!   单个格式问题不是错误，而是 `models::Violation`。
//! - 面向用户的中文文案通过 [`CoreError::user_message`] 获取，不含技术术语堆砌。
//! - IO / Zip / XML 底层错误一律装箱为 Source 变体，保留 cause 链。

use std::fmt;

/// 核心引擎统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 读取或写入文件失败。
    #[error("IO 错误: {context}: {source}")]
    Io {
        /// 出错时的操作上下文（如“打开论文文件”）。
        context: String,
        #[source]
        source: std::io::Error,
    },

    /// zip 解包 .docx 失败（含“不是合法 zip”即非 docx 文件）。
    #[error("压缩包读取失败: {0}")]
    Zip(#[from] ZipError),

    /// OOXML（document.xml / styles.xml 等）不是良构 XML。
    #[error("文档内部 XML 格式损坏: {part}")]
    Xml {
        /// 出错的部件名，如 "word/document.xml"。
        part: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// .docx 缺少必需部件（例如没有 word/document.xml）。
    #[error("文件缺少必需的文档部件: {part}（可能不是有效的 Word 文档）")]
    MissingPart { part: String },

    /// 规则配置文件（JSON/YAML）语法错误。
    #[error("规则配置文件语法错误: {path}")]
    RuleSyntax {
        path: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// 规则配置语义校验失败（ID 重复、期望值缺失等）。
    #[error("规则配置不合法: {reason}")]
    RuleInvalid { reason: String },

    /// 样式 basedOn 链出现循环引用，无法计算有效格式。
    #[error("样式继承链存在循环引用: {cycle}")]
    StyleCycle {
        /// 便于排查的链路描述，如 "A -> B -> A"。
        cycle: String,
    },

    /// 引用了样式表中不存在的样式 ID（宽容处理：可降级为警告时使用 None）。
    #[error("引用了不存在的样式: {style_id}")]
    UnknownStyle { style_id: String },

    /// 数值超出合理范围（字号、行距、页边距等）。
    #[error("数值超出合理范围: {field} = {value}（允许范围 {allowed}）")]
    OutOfRange {
        field: String,
        value: String,
        allowed: String,
    },

    /// 输入路径不存在或不是目录/文件。
    #[error("路径无效: {path}（{reason}）")]
    InvalidPath { path: String, reason: String },
}

/// 轻量 zip 错误包装：避免在 error 层直接暴露第三方库版本细节。
#[derive(Debug)]
pub struct ZipError {
    pub message: String,
}

impl fmt::Display for ZipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ZipError {}

impl From<std::io::Error> for ZipError {
    fn from(e: std::io::Error) -> Self {
        Self {
            message: e.to_string(),
        }
    }
}

impl CoreError {
    /// 构造 IO 错误。
    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// 构造规则配置语义错误。
    pub fn rule_invalid(reason: impl Into<String>) -> Self {
        Self::RuleInvalid {
            reason: reason.into(),
        }
    }

    /// 面向非技术人员的中文提示（用于 GUI 弹窗/报告页脚）。
    pub fn user_message(&self) -> String {
        match self {
            CoreError::Io { context, .. } => format!("文件读写失败（{context}），请检查文件是否被其他程序占用。"),
            CoreError::Zip(_) => "该文件无法打开，可能已损坏或不是有效的 .docx 文档。".into(),
            CoreError::Xml { .. } => "文档内部数据异常，Word 可能可以修复它；若反复失败，请在 Word 中另存为新文件后再审核。".into(),
            CoreError::MissingPart { .. } => "该文件不像是一个完整的 Word 文档，请确认扩展名为 .docx 且未损坏。".into(),
            CoreError::RuleSyntax { path, .. } => format!("格式规范文件“{path}”内容有格式错误，请检查括号、引号是否成对。"),
            CoreError::RuleInvalid { reason } => format!("格式规范设置有误：{reason}。"),
            CoreError::StyleCycle { .. } => "文档里的文字样式互相引用形成了死循环，无法判断实际格式，建议在 Word 中重置样式。".into(),
            CoreError::UnknownStyle { style_id } => format!("文档引用了一个不存在的文字样式（{style_id}），相关段落按默认格式处理。"),
            CoreError::OutOfRange { field, value, .. } => format!("设置值不合理：{field} = {value}，请修改格式规范文件。"),
            CoreError::InvalidPath { path, .. } => format!("找不到“{path}”，请重新选择文件夹或文件。"),
        }
    }
}

/// 核心引擎统一的 Result 别名。
pub type CoreResult<T> = Result<T, CoreError>;

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_error_implements_std_error() {
        let e = CoreError::rule_invalid("规则 ID 重复");
        // thiserror 生成的 Display 与 Error trait 均可用。
        let boxed: Box<dyn std::error::Error> = Box::new(e);
        assert!(boxed.to_string().contains("规则 ID 重复"));
    }

    #[test]
    fn io_error_keeps_source_chain() {
        let raw = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let e = CoreError::io("打开论文文件", raw);
        assert!(matches!(e, CoreError::Io { .. }));
        assert!(std::error::Error::source(&e).is_some());
    }

    #[test]
    fn user_messages_are_chinese_and_actionable() {
        let cases = vec![
            (CoreError::Zip(ZipError { message: "bad".into() }), "不是有效的"),
            (
                CoreError::MissingPart {
                    part: "word/document.xml".into(),
                },
                "完整的 Word 文档",
            ),
            (
                CoreError::StyleCycle {
                    cycle: "A -> B -> A".into(),
                },
                "死循环",
            ),
            (
                CoreError::InvalidPath {
                    path: "/tmp/x".into(),
                    reason: "不存在".into(),
                },
                "找不到",
            ),
        ];
        for (e, needle) in cases {
            let msg = e.user_message();
            assert!(msg.contains(needle), "expect {needle:?} in {msg:?}");
            // 全部为可读中文句子（以句号结尾即可粗略保证非空且有标点）。
            assert!(msg.ends_with('。'));
        }
    }

    #[test]
    fn out_of_range_displays_field_and_value() {
        let e = CoreError::OutOfRange {
            field: "font_size_pt".into(),
            value: "72".into(),
            allowed: "5..=72".into(),
        };
        let s = e.to_string();
        assert!(s.contains("font_size_pt") && s.contains("72"));
    }

    #[test]
    fn core_result_alias_works() {
        fn ok_fn() -> CoreResult<u32> {
            Ok(7)
        }
        fn err_fn() -> CoreResult<u32> {
            Err(CoreError::UnknownStyle {
                style_id: "Foo".into(),
            })
        }
        assert_eq!(ok_fn().unwrap(), 7);
        assert!(err_fn().unwrap_err().to_string().contains("Foo"));
    }
}
