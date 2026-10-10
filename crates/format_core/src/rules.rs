//! rules —— 规则配置加载与定义（占位）。
//!
//! P3 将实现：从 JSON/YAML 加载规则集（字号、字体、行距、加粗、
//! 标题层级、页边距、标点、图表编号等），运行时零自然语言模型。

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
