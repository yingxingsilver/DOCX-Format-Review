//! audit —— 规则执行引擎（占位）。
//!
//! P4 将实现：对单文档执行全部规则；批量审核基于 rayon 并行。

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
