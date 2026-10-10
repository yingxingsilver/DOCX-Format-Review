//! rules —— 规则配置加载、校验与 profile 应用。
//!
//! - [`load_rule_config`]：从 JSON 文件加载（含正则预编译校验，失败给出中文提示）。
//! - [`apply_profile`]：strict / normal / loose 预设，支持 overrides（severity/enabled/manual_review）与 disabled。

use std::collections::HashSet;
use std::path::Path;

use regex::Regex;

use crate::dsl::{Rule, RuleConfig};
use crate::error::{CoreError, CoreResult};

/// 支持的审核力度预设。
pub const KNOWN_PROFILES: [&str; 3] = ["strict", "normal", "loose"];

/// 从 JSON 字符串解析并做语义校验。
pub fn parse_rule_config(json: &str, source_path: &str) -> CoreResult<RuleConfig> {
    let config: RuleConfig = serde_json::from_str(json).map_err(|e| CoreError::RuleSyntax {
        path: source_path.to_string(),
        source: Box::new(e),
    })?;
    validate_rule_config(&config)?;
    Ok(config)
}

/// 从文件加载规则配置。错误文案面向普通用户。
pub fn load_rule_config(path: impl AsRef<Path>) -> CoreResult<RuleConfig> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(CoreError::InvalidPath {
            path: path.display().to_string(),
            reason: "规则文件不存在".into(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|e| CoreError::Io {
        context: format!("读取规则文件“{}”", path.file_name().unwrap_or_default().to_string_lossy()),
        source: e,
    })?;
    parse_rule_config(&text, &path.display().to_string())
}

/// 语义校验：ID 唯一、正则可编译、roles 合法。
pub fn validate_rule_config(config: &RuleConfig) -> CoreResult<()> {
    let mut seen: HashSet<String> = HashSet::new();
    for rule in &config.rules {
        if !seen.insert(rule.id.clone()) {
            return Err(CoreError::RuleInvalid {
                reason: format!("存在重复的规则编号“{}”", rule.id),
            });
        }
        precompile_regexes_of(rule)?;
        if let Some(t) = &rule.target {
            check_role_names(&t.roles, &rule.id)?;
            check_role_names(&t.exclude_roles, &rule.id)?;
        }
    }
    Ok(())
}

fn check_role_names(roles: &[String], rule_id: &str) -> CoreResult<()> {
    for r in roles {
        if crate::dsl::ParagraphRole::from_kind_str(r).is_none() {
            return Err(CoreError::RuleInvalid {
                reason: format!("规则“{rule_id}”引用了未知的段落类型“{r}”"),
            });
        }
    }
    Ok(())
}

/// 预编译规则中出现的所有正则；任一失败 → InvalidRegex（加载期报错，运行期不再处理）。
fn precompile_regexes_of(rule: &Rule) -> CoreResult<()> {
    use crate::dsl::RuleKind as K;
    let mut patterns: Vec<&str> = Vec::new();
    match &rule.kind {
        K::Caption { pattern, .. } => patterns.push(pattern),
        K::ReferenceList { entry_pattern, .. } => patterns.push(entry_pattern),
        K::Punctuation { exceptions, .. } => patterns.extend(exceptions.iter().map(|s| s.as_str())),
        K::Sequence { anchors, .. } => patterns.extend(anchors.iter().filter_map(|a| a.regex.as_deref())),
        _ => {}
    }
    if let Some(t) = &rule.target {
        if let Some(re) = &t.text_regex {
            patterns.push(re);
        }
    }
    for p in patterns {
        Regex::new(p).map_err(|e| CoreError::InvalidRegex { rule_id: rule.id.clone(), source: e })?;
    }
    Ok(())
}

/// 应用 profile：覆盖 severity / enabled / manual_review，并按 disabled 列表禁用规则。
///
/// - `profile_name` 必须存在于 config.profiles（或为 "normal" 的空默认），否则 UnknownProfile。
/// - 引用了不存在的规则 id → RuleInvalid（配置错误应在加载期暴露）。
pub fn apply_profile(config: &RuleConfig, profile_name: &str) -> CoreResult<RuleConfig> {
    let profile = config
        .profiles
        .get(profile_name)
        .cloned()
        .or_else(|| if profile_name == "normal" { Some(Default::default()) } else { None })
        .ok_or_else(|| CoreError::UnknownProfile { profile: profile_name.to_string() })?;

    let mut out = config.clone();
    for ov in &profile.overrides {
        let target = out
            .rules
            .iter_mut()
            .find(|r| r.id == ov.id)
            .ok_or_else(|| CoreError::RuleInvalid {
                reason: format!("预设“{profile_name}”引用了不存在的规则编号“{}”", ov.id),
            })?;
        if let Some(sev) = ov.severity {
            target.severity = sev;
        }
        if let Some(enabled) = ov.enabled {
            target.enabled = enabled;
        }
        if let Some(mr) = ov.manual_review {
            target.manual_review = mr;
        }
    }
    for id in &profile.disabled {
        let exists = out.rules.iter().any(|r| r.id == *id);
        if !exists {
            return Err(CoreError::RuleInvalid {
                reason: format!("预设“{profile_name}”禁用了不存在的规则编号“{id}”"),
            });
        }
        if let Some(r) = out.rules.iter_mut().find(|r| r.id == *id) {
            r.enabled = false;
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests_helper {
    /// 测试辅助：从内嵌 JSON 构造配置（跳过文件 IO）。
    pub fn config_from_json(json: &str) -> crate::dsl::RuleConfig {
        super::parse_rule_config(json, "<test>").expect("测试用规则配置应能解析")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::Severity;

    fn sample_json() -> String {
        include_str!("../../../resources/rules/education_generic.json").to_string()
    }

    #[test]
    fn loads_shipped_rules_json() {
        let cfg = load_rule_config("resources/rules/education_generic.json").unwrap();
        assert_eq!(cfg.rules.len(), 40);
        assert_eq!(cfg.defaults.tolerance.font_size_pt, 0.5);
        assert_eq!(cfg.font_size_alias_cn(12.0), Some("小四"));
    }

    #[test]
    fn rejects_duplicate_rule_ids() {
        let mut cfg = parse_rule_config(&sample_json(), "<test>").unwrap();
        let r = cfg.rules[0].clone();
        cfg.rules.push(r);
        let e = validate_rule_config(&cfg).unwrap_err();
        assert!(matches!(e, CoreError::RuleInvalid { .. }));
        assert!(e.user_message().contains("重复"));
    }

    #[test]
    fn invalid_regex_reported_at_load_time_with_chinese_hint() {
        let json = r#"{
            "version":"1","name":"t",
            "defaults":{"tolerance":{}},
            "target_inference":{"figure_caption_regex":"(","table_caption_regex":"表","reference_entry_regex":"\\\\[","in_text_citation_regex":"\\\\"},
            "rules":[{"id":"x","severity":"warning","message":"m","type":"caption","caption_kind":"figure","pattern":"("}]
        }"#;
        let e = parse_rule_config(json, "<test>").unwrap_err();
        assert!(matches!(e, CoreError::InvalidRegex { .. }), "got {e:?}");
        assert!(e.user_message().contains("正则表达式无效"));
    }

    #[test]
    fn strict_profile_upgrades_severity() {
        let cfg = parse_rule_config(&sample_json(), "<test>").unwrap();
        let strict = apply_profile(&cfg, "strict").unwrap();
        let ls = strict.rules.iter().find(|r| r.id == "body-line-spacing").unwrap();
        assert_eq!(ls.severity, Severity::Error);
        // normal 保持原样
        let normal = apply_profile(&cfg, "normal").unwrap();
        let ls = normal.rules.iter().find(|r| r.id == "body-line-spacing").unwrap();
        assert_eq!(ls.severity, Severity::Warning);
    }

    #[test]
    fn loose_profile_disables_rules() {
        let cfg = parse_rule_config(&sample_json(), "<test>").unwrap();
        let loose = apply_profile(&cfg, "loose").unwrap();
        assert!(!loose.rules.iter().find(|r| r.id == "abstract-length").unwrap().enabled);
        assert!(!loose.rules.iter().find(|r| r.id == "cjk-latin-space").unwrap().enabled);
    }

    #[test]
    fn unknown_profile_gives_friendly_error() {
        let cfg = parse_rule_config(&sample_json(), "<test>").unwrap();
        let e = apply_profile(&cfg, "turbo").unwrap_err();
        assert!(matches!(e, CoreError::UnknownProfile { .. }));
        assert!(e.user_message().contains("预设"));
    }

    #[test]
    fn override_unknown_rule_id_is_invalid() {
        let json = r#"{
            "version":"1","name":"t",
            "defaults":{"tolerance":{}},
            "target_inference":{"figure_caption_regex":"图","table_caption_regex":"表","reference_entry_regex":"x","in_text_citation_regex":"y"},
            "profiles":{"strict":{"overrides":[{"id":"ghost","severity":"error"}],"disabled":[]}},
            "rules":[{"id":"x","severity":"warning","message":"m","type":"bold","expected":true}]
        }"#;
        let cfg = parse_rule_config(json, "<test>").unwrap();
        let e = apply_profile(&cfg, "strict").unwrap_err();
        assert!(matches!(e, CoreError::RuleInvalid { .. }));
    }
}
