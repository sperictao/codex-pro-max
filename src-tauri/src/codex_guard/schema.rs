//! schema：内置 + 磁盘托管参数的加载、合并与按界面语言取值

use std::path::PathBuf;

use crate::config;
use crate::i18n::trf;

use super::GuardParam;

const BUILTIN_SCHEMA: &str = include_str!("guard_schema.json");

/// 按界面语言挑文案：en 且有英文资源时用英文，否则用原文（纯函数，便于测试）
pub(crate) fn pick_i18n<'a>(primary: &'a str, en: &'a str, lang: &str) -> &'a str {
    if lang == "en" && !en.is_empty() {
        en
    } else {
        primary
    }
}

/// 按界面语言挑 default 值：en 且有英文内容时用英文，否则用原文（纯函数，便于测试）
pub(crate) fn default_for_lang<'a>(p: &'a GuardParam, lang: &str) -> &'a serde_json::Value {
    if lang == "en" && !p.default_en.is_null() {
        &p.default_en
    } else {
        &p.default
    }
}

pub(crate) fn schema_file_path() -> Result<PathBuf, String> {
    Ok(config::home_dir()?
        .join(".dashi-taskboard-launcher")
        .join("codex-guard-schema.json"))
}

/// 加载 schema：磁盘同 id 覆盖内置（用户定制内置参数），磁盘独有条目保留；
/// 例外：label_en/description_en/default_en 是随二进制发布的英文资源、不算用户数据，
/// 始终以内置为准——否则旧版本释放到磁盘的中文副本会永久滞留（i18n 实机踩坑）
pub(crate) fn load_schema() -> Vec<GuardParam> {
    let builtin: Vec<GuardParam> =
        serde_json::from_str(BUILTIN_SCHEMA).expect("内置 guard schema 必须可解析");

    let path = match schema_file_path() {
        Ok(p) => p,
        Err(_) => return builtin,
    };
    if !path.exists() {
        // 首次运行释放默认 schema，方便用户自行扩展
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, BUILTIN_SCHEMA);
        return builtin;
    }
    let disk: Vec<GuardParam> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();

    merge_schema(builtin, disk)
}

/// 合并内置与磁盘 schema（纯函数，便于测试）
fn merge_schema(builtin: Vec<GuardParam>, disk: Vec<GuardParam>) -> Vec<GuardParam> {
    let mut merged = builtin;
    for mut d in disk {
        if let Some(slot) = merged.iter().position(|m| m.id == d.id) {
            // 英文资源以内置条目为准，其余字段尊重磁盘定制
            d.label_en = merged[slot].label_en.clone();
            d.description_en = merged[slot].description_en.clone();
            d.default_en = merged[slot].default_en.clone();
            merged[slot] = d;
        } else {
            merged.push(d); // 磁盘独有条目保留
        }
    }
    merged
}

/// 加载磁盘上的用户 schema（不含内置合并）
pub(crate) fn load_disk_schema() -> Result<Vec<GuardParam>, String> {
    let path = schema_file_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| trf("Failed to read schema file: {error}", &[("error", e.to_string())]))?;
    let schema: Vec<GuardParam> = serde_json::from_str(&content)
        .map_err(|e| trf("Failed to parse schema file: {error}", &[("error", e.to_string())]))?;
    Ok(schema)
}

pub(crate) fn save_disk_schema(schema: &[GuardParam]) -> Result<(), String> {
    let path = schema_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| trf("Failed to create schema directory: {error}", &[("error", e.to_string())]))?;
    }
    let content = serde_json::to_string_pretty(schema)
        .map_err(|e| trf("Failed to serialize schema: {error}", &[("error", e.to_string())]))?;
    std::fs::write(&path, content)
        .map_err(|e| trf("Failed to write schema file: {error}", &[("error", e.to_string())]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_schema_parses() {
        let v: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        assert_eq!(v.len(), 11);
    }

    #[test]
    fn builtin_schema_has_english_labels() {
        // 内置参数必须带英文资源，否则英文界面落回中文原文
        let v: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        for p in &v {
            assert!(!p.label_en.is_empty(), "{} 缺 label_en", p.id);
            assert!(!p.description_en.is_empty(), "{} 缺 description_en", p.id);
        }
    }

    #[test]
    fn pick_i18n_selects_by_language() {
        // en 有英文资源 → 英文；无英文资源（自定义参数）→ 落回原文；zh → 原文
        assert_eq!(pick_i18n("中文", "English", "en"), "English");
        assert_eq!(pick_i18n("自定义", "", "en"), "自定义");
        assert_eq!(pick_i18n("中文", "English", "zh-CN"), "中文");
    }

    #[test]
    fn merge_schema_keeps_builtin_english_resources() {
        // 回归：旧版磁盘副本无 label_en，同 id 覆盖内置时不得盖掉英文资源
        let json = |id: &str, label: &str, label_en: &str, default_en: serde_json::Value| {
            serde_json::from_value::<GuardParam>(serde_json::json!({
                "id": id, "label": label, "label_en": label_en,
                "file": "config.toml", "apply_mode": "toml_key",
                "default_en": default_en,
            }))
            .unwrap()
        };
        let builtin = vec![json("a", "内置中文", "Builtin EN", serde_json::json!("EN content"))];
        let disk = vec![
            // 同 id 覆盖：label 尊重磁盘，英文资源（含 default_en）以内置为准
            json("a", "用户改过的中文", "", serde_json::Value::Null),
            json("custom.x", "自定义", "", serde_json::Value::Null), // 磁盘独有：保留
        ];
        let merged = merge_schema(builtin, disk);
        assert_eq!(merged.len(), 2);
        let a = merged.iter().find(|p| p.id == "a").unwrap();
        assert_eq!(a.label, "用户改过的中文"); // 磁盘定制生效
        assert_eq!(a.label_en, "Builtin EN"); // 英文资源以内置为准
        assert_eq!(a.default_en, serde_json::json!("EN content"));
        assert!(merged.iter().any(|p| p.id == "custom.x"));
    }

    #[test]
    fn default_for_lang_selects_content_by_language() {
        let p = |default_en: serde_json::Value| GuardParam {
            id: "x".into(),
            label: "中文".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: "f".into(),
            apply_mode: "file_overwrite".into(),
            path: String::new(),
            value_type: "text".into(),
            default: serde_json::json!("中文内容"),
            default_en,
            custom: false,
        };
        // en + 有英文内容 → 英文；en + 无英文内容 → 原文；zh → 原文
        let with_en = p(serde_json::json!("English content"));
        assert_eq!(default_for_lang(&with_en, "en"), &serde_json::json!("English content"));
        assert_eq!(default_for_lang(&with_en, "zh-CN"), &serde_json::json!("中文内容"));
        let without_en = p(serde_json::Value::Null);
        assert_eq!(default_for_lang(&without_en, "en"), &serde_json::json!("中文内容"));
    }

    #[test]
    fn builtin_schema_has_no_custom_flag() {
        let builtin: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        for p in &builtin {
            assert!(!p.custom, "内置参数 {} 不应有 custom=true", p.id);
        }
    }
}
