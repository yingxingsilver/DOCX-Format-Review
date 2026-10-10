//! docx_parser —— OOXML 解包与解析。
//!
//! 当前阶段（规则引擎先行）提供**可替换的解析入口**：
//! - [`parse_docx`]：读取 .docx 的最小校验（存在、非空、zip 魔数 PK\x03\x04），
//!   并尝试加载同目录/同名 `.parsed.json`（由样式继承求解器产出的 [`ParsedDocument`]）。
//!   真正的 zip + quick-xml 全量解析在 P2 落地，接口保持不变。
//! - 损坏 docx / 加密 docx / 空文件 → `CoreError::Zip/MissingPart/Io`，
//!   上层 audit 会转成 `FileStatus::ReadFailed` + 中文友好文案，不中断批量任务。

use std::path::Path;

use crate::dsl::ParsedDocument;
use crate::error::{CoreError, CoreResult};

/// 骨架自检占位（保持与 lib.rs 模块链接测试兼容）。
pub fn placeholder_ok() -> bool {
    true
}

/// 解析一个 .docx 文件为 [`ParsedDocument`]。
///
/// 第一版策略：
/// 1. 文件不存在 → InvalidPath（“找不到这份文件”）。
/// 2. 空文件 → Zip（“这份文件是空的，可能尚未保存完成。”）。
/// 3. 无 zip 魔数（PK）→ Zip（损坏或加密的提示）。
/// 4. 若存在同名 `.parsed.json`（解析+样式求解管线的产物），直接反序列化返回。
/// 5. 否则 MissingPart（“暂不支持直接读取该文档内容”——P2 前由管线 JSON 驱动审核）。
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
    if bytes.is_empty() {
        return Err(CoreError::Zip {
            note: "这份文件是空的，可能尚未保存完成。".into(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty file",
            )),
        });
    }
    if !(bytes.len() >= 2 && bytes[0] == b'P' && bytes[1] == b'K') {
        // 非 zip 容器：可能是损坏文件，也可能是旧版 .doc / 受密码保护后重命名的文件。
        return Err(CoreError::Zip {
            note: "这份文件无法识别，可能已损坏、是旧版格式或受密码保护。".into(),
            source: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a zip container",
            )),
        });
    }
    // 优先读取解析管线产物 <file>.parsed.json（存在即视为已完成 DOCX→DSL 转换）。
    let json_path = sidecar_json(path);
    if json_path.exists() {
        let text = std::fs::read_to_string(&json_path).map_err(|e| CoreError::Io {
            context: "读取解析结果".into(),
            source: e,
        })?;
        let mut doc: ParsedDocument =
            serde_json::from_str(&text).map_err(|e| CoreError::Xml {
                part: "document".into(),
                source: Box::new(e),
            })?;
        // 以真实输入路径为准（sidecar 中可能记录的是导出机路径）。
        doc.path = path.to_path_buf();
        doc.file_name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        return Ok(doc);
    }
    Err(CoreError::MissingPart {
        part: "word/document.xml".into(),
    })
}

/// sidecar 路径：`foo.docx` → `foo.docx.parsed.json`。
fn sidecar_json(path: &Path) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".parsed.json");
    std::path::PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("fmt_docx_parser_tests");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(name)
    }

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }

    #[test]
    fn missing_file_is_invalid_path() {
        let e = parse_docx(&tmp("nope.docx")).unwrap_err();
        assert!(matches!(e, CoreError::InvalidPath { .. }));
        assert!(!e.user_message().is_empty());
    }

    #[test]
    fn empty_file_is_friendly_zip_error() {
        let p = tmp("empty.docx");
        std::fs::write(&p, b"").unwrap();
        let e = parse_docx(&p).unwrap_err();
        assert!(matches!(e, CoreError::Zip { .. }));
        let msg = e.user_message();
        assert!(msg.contains("空"), "{msg}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn non_zip_bytes_report_corrupted_not_technical() {
        let p = tmp("broken.docx");
        std::fs::write(&p, b"this is definitely not a docx").unwrap();
        let e = parse_docx(&p).unwrap_err();
        let msg = e.user_message().to_ascii_lowercase();
        assert!(msg.contains("损坏") || msg.contains("密码"), "{msg}");
        for banned in ["xml", "w:", "panic", "unwrap", "stack"] {
            assert!(!msg.contains(banned));
        }
        let _ = std::fs::remove_file(&p);
    }
}
