//! error —— 核心引擎统一错误类型。
//!
//! 约定：
//! - `CoreError` 表达“整个文件/整次运行失败”的错误（无法解析、配置非法等）；
//!   单个格式问题不是错误，而是 `dsl::Violation`。
//! - 面向用户的中文文案通过 [`CoreError::user_message`] 获取，不含技术术语堆砌，
//!   不出现 XML 节点名、内部路径、stack trace。
//! - IO / Zip / XML 底层错误一律装箱为 Source 变体，保留 cause 链供日志使用。

use std::fmt::Write as _;

/// 核心引擎统一错误类型。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 读取或写入文件失败。
    #[error("IO 错误: {context}")]
    Io {
        /// 出错时的操作上下文（如“打开论文文件”）。
        context: String,
        #[source]
        source: std::io::Error,
    },

    /// zip 解包 .docx 失败（含“不是合法 zip”即非 docx 文件）。
    #[error("文档压缩包无法读取")]
    Zip {
        /// 面向用户的补充说明（可为空）。
        note: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// OOXML（document.xml / styles.xml 等）不是良构 XML。
    #[error("文档内部内容损坏: {part}")]
    Xml {
        /// 出错的部件名，如 "word/document.xml"。
        part: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// .docx 缺少必需部件（例如没有 word/document.xml）。
    #[error("文件缺少必需的文档部件: {part}")]
    MissingPart { part: String },

    /// 规则配置文件（JSON/YAML）语法错误。
    #[error("规则配置文件格式有误: {path}")]
    RuleSyntax {
        path: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// 规则配置语义校验失败（ID 重复、期望值缺失等）。
    #[error("规则配置不合法: {reason}")]
    RuleInvalid { reason: String },

    /// 规则文件中的正则表达式无法编译。
    #[error("规则文件中的正则表达式无效: {rule_id}")]
    InvalidRegex {
        rule_id: String,
        #[source]
        source: regex::Error,
    },

    /// 样式 basedOn 链出现循环引用，无法计算有效格式。
    #[error("样式继承链存在循环引用: {cycle}")]
    StyleCycle { cycle: String },

    /// 引用了样式表中不存在的样式 ID。
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

    /// 请求的规则 profile 不存在。
    #[error("未找到审核力度预设: {profile}")]
    UnknownProfile { profile: String },
}

/// 轻量结果别名。
pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    /// 面向普通用户的中文提示。禁止出现 XML 节点名 / stack trace / 内部实现词。
    pub fn user_message(&self) -> String {
        match self {
            CoreError::Io { .. } => {
                "文件读取失败，请确认文件没有被其他程序占用，且未被加密。".to_string()
            }
            CoreError::Zip { .. } => {
                "无法读取该 Word 文档，文件可能已损坏或不是有效的 .docx 文档。".to_string()
            }
            CoreError::Xml { .. } => {
                "文档内部内容损坏，Word 可能提示需要修复；建议另存为新文件后重试。".to_string()
            }
            CoreError::MissingPart { .. } => {
                "文件缺少必要的文档内容，可能不是有效的 Word 论文文档。".to_string()
            }
            CoreError::RuleSyntax { .. } => {
                "规则配置文件格式有误，请检查其中的标点或拼写。".to_string()
            }
            CoreError::RuleInvalid { reason } => format!("规则配置不合法：{reason}"),
            CoreError::InvalidRegex { rule_id, .. } => {
                format!(
                    "规则文件中的正则表达式无效（规则编号 {rule_id}），请联系规则维护人员修正。"
                )
            }
            CoreError::StyleCycle { .. } => {
                "文档样式之间存在相互引用，部分格式未能自动判断，建议人工检查。".to_string()
            }
            CoreError::UnknownStyle { .. } => {
                "文档中引用了不存在的样式，部分格式未能自动判断，建议人工检查。".to_string()
            }
            CoreError::OutOfRange { .. } => {
                "文档中存在异常的尺寸数值，相关项目已降级为人工检查。".to_string()
            }
            CoreError::InvalidPath { .. } => "所选文件或文件夹不存在，请重新选择。".to_string(),
            CoreError::UnknownProfile { profile } => {
                format!("没有找到名为“{profile}”的审核力度预设。")
            }
        }
    }

    /// 调试用完整信息（含 cause 链），仅用于日志文件，不展示给普通用户。
    pub fn debug_chain(&self) -> String {
        let mut s = format!("{self}");
        let mut src = std::error::Error::source(self);
        while let Some(e) = src {
            let _ = write!(s, "\n  caused by: {e}");
            src = e.source();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_is_chinese_and_free_of_technical_terms() {
        let e = CoreError::Zip {
            note: String::new(),
            source: Box::new(std::io::Error::new(std::io::ErrorKind::Other, "bad zip")),
        };
        let msg = e.user_message();
        assert!(msg.contains("Word 文档"));
        for banned in ["w:sz", "pPr", "XML parse", "unwrap", "rayon", "IPC"] {
            assert!(!msg.contains(banned), "用户文案不应包含技术词 {banned}");
        }
    }

    #[test]
    fn invalid_regex_message_names_rule_id() {
        // 用非字面量构造非法正则，避免 clippy 在编译期直接拒绝该模式。
        let bad_pattern = String::from("((");
        let re = regex::Regex::new(&bad_pattern)
            .err()
            .expect("非法正则应返回错误");
        let e = CoreError::InvalidRegex {
            rule_id: "figure-caption".into(),
            source: re,
        };
        assert!(e.user_message().contains("figure-caption"));
        assert!(e.user_message().contains("正则表达式无效"));
    }

    #[test]
    fn debug_chain_keeps_source() {
        let e = CoreError::Io {
            context: "打开论文文件".into(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert!(e.debug_chain().contains("PermissionDenied") || e.debug_chain().contains("denied"));
    }
}
