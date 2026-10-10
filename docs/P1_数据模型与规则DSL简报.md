# P1 修改简报 —— 核心数据模型与规则 DSL

日期：2026-10-10　分支：main（发布仓库 DOCX-Format-Review）

## 范围
仅数据模型层（crates/format_core/src/models.rs、error.rs），不含 DOCX 解析、不含 GUI、不引入 Tauri。

## 变更文件
- crates/format_core/src/models.rs（重写占位为完整模型）
  - DocumentModel / SectionProperty / Paragraph / Run / Style / EffectiveFormat / Violation / AuditReport / SummaryCounts
  - 枚举：Severity、LineSpacingRule、Alignment、PageOrientation、ParagraphSource、StyleKind、RuleScope、CaptionNumbering
  - 规则 DSL：RuleConfig / Tolerance / RuleDef + RuleBody（9 种静态规则类型：font_size、font_family、line_spacing、bold、italic、heading_style、margin、punctuation、caption_number；serde tag="type" + flatten，JSON 友好）
  - 单位约定：字号统一 pt（RunProps::half_points_to_pt 为唯一半磅换算入口）；边距/行距/缩进统一 pt；页宽高等保留 twips 原语义
- crates/format_core/src/error.rs（重写占位）
  - CoreError（thiserror）：Io / Zip / Xml / MissingPart / RuleSyntax / RuleInvalid / StyleCycle / UnknownStyle / OutOfRange / InvalidPath
  - user_message() 输出面向非技术人员的中文提示；CoreResult<T> 别名
- crates/format_core/Cargo.toml：启用 serde、thiserror；serde_json 移至 dev-dependencies（P3 加载器时转正式依赖）
- crates/format_core/src/lib.rs：更新模块链接测试；新增 shipped_default_rules_match_schema 验收测试（真实解析 rules/default.json）
- rules/default.json（新增）：本科毕业论文示例规范，9 条规则——正文小四 12pt、中文宋体/西文 Times New Roman、行距固定值 20pt、一级标题 Heading1 三号 16pt 黑体加粗、页边距、标点、图/表题注编号（章-序号）

## 关键设计决策（假设已按此固化）
1. bold/italic/underline 在 EffectiveFormat 中用 Option<bool> 三态（用户要求 bool，但 OOXML "0"/"false" 与“未设置”必须可区分；None=整链未设置）。若需严格 bool 可改为默认 false，请在 P4 前指出。
2. LineSpacingRule.AtLeast=最小值、Exactly=固定值（对应 w:lineRule 原文），注释已标明，避免常见混淆。
3. generated_at 用 RFC3339 字符串，避免引入 chrono。
4. 单个格式问题不是错误而是 Violation；CoreError 只表达整文件/整次运行失败。

## 测试
cargo test -p format_core → 28 passed / 0 failed（models 15、error 5、lib 3、其余占位模块 5）。
覆盖：半磅→pt、twips→pt、JSON 双向序列化、RuleConfig 解析示例文件、规则 ID 去重校验、SummaryCounts 统计、user_message 中文文案。

## 后续衔接
P2 docx_parser 填充 DocumentModel；style_resolver 产出 EffectiveFormat；P3 rules.rs 实现 RuleConfig 的 JSON/YAML 加载与语义校验；P4 audit 消费 RuleBody 枚举执行检查。
