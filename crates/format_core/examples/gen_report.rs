//! P6 演示：生成样例 HTML/CSV 审核报告，便于在浏览器中人工验收样式。
//!
//! 运行：`cargo run -p format_core --example gen_report`
//! 产物：系统临时目录下的 report_demo.html / report_demo.csv

use std::path::Path;

fn main() {
    let dir = std::env::temp_dir().join("format-core-report-demo");
    std::fs::create_dir_all(&dir).expect("创建演示输出目录失败");
    let report = format_core::report::sample_demo_report();
    format_core::report::generate_html_report(&report, Path::new(&dir).join("report_demo.html"))
        .expect("生成 HTML 失败");
    format_core::report::generate_csv_report(&report, Path::new(&dir).join("report_demo.csv"))
        .expect("生成 CSV 失败");
    println!("演示报告已生成：{}", dir.display());
}
