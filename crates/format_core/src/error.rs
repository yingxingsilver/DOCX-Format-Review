//! error —— 统一错误类型（占位）。
//!
//! P1 将基于 `thiserror` 定义 `CoreError`：
//! Zip/IO/XML 解析错误、规则配置错误、样式循环引用等。

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
