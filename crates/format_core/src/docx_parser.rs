//! docx_parser —— OOXML 解包与解析（占位）。
//!
//! P1 将基于 `zip` + `quick-xml` 实现：
//! - 打开 .docx（OPC zip 容器）
//! - 读取 word/document.xml、word/styles.xml、word/settings.xml
//! - 提取段落 / Run / 表格 / 分节符等结构

/// 骨架占位，供 lib.rs 的模块链接测试使用。
pub fn placeholder_ok() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_exists() {
        assert!(placeholder_ok());
    }
}
