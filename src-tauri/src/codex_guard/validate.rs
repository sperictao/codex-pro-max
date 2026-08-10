//! 自定义参数与看守文件的输入校验（纯函数）

use crate::i18n::{tr, trf};

use super::{GuardFile, GuardFileFormat, GuardParam};

const CUSTOM_ID_PREFIX: &str = "custom.";

pub(crate) fn normalize_custom_id(id: &str) -> String {
    if id.starts_with(CUSTOM_ID_PREFIX) {
        id.to_string()
    } else {
        format!("{}{}", CUSTOM_ID_PREFIX, id)
    }
}

pub(crate) fn validate_file_path(rel: &str) -> Result<(), String> {
    if rel.trim().is_empty() {
        return Err(tr("File path cannot be empty"));
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(tr(
            "File path must be relative to ~/.codex and cannot start with /",
        ));
    }
    for seg in rel.split(['/', '\\']) {
        if seg == ".." {
            return Err(tr(
                "File path cannot contain .. and must stay inside ~/.codex",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_guard_file(f: &GuardFile) -> Result<(), String> {
    if f.name.trim().is_empty() {
        return Err(tr("File name cannot be empty"));
    }
    validate_file_path(&f.file)?;
    Ok(())
}

pub(crate) fn validate_param_for_file(
    p: &GuardParam,
    format: GuardFileFormat,
) -> Result<(), String> {
    match (format, p.apply_mode.as_str()) {
        (GuardFileFormat::Toml, "toml_key" | "toml_absent")
        | (GuardFileFormat::Markdown, "markdown_block")
        | (_, "file_overwrite") => Ok(()),
        _ => Err(trf(
            "Apply mode {mode} is incompatible with file format {format}",
            &[
                ("mode", p.apply_mode.clone()),
                ("format", format.to_string()),
            ],
        )),
    }
}

fn validate_apply_mode(mode: &str) -> Result<(), String> {
    match mode {
        "toml_key" | "toml_absent" | "file_overwrite" | "markdown_block" => Ok(()),
        other => Err(trf(
            "Unsupported apply_mode: {mode}",
            &[("mode", other.to_string())],
        )),
    }
}

fn validate_value_type(value_type: &str) -> Result<(), String> {
    match value_type {
        "bool" | "int" | "string" | "text" | "none" => Ok(()),
        other => Err(trf(
            "Unsupported value_type: {type}",
            &[("type", other.to_string())],
        )),
    }
}

pub(crate) fn validate_param_fields(p: &GuardParam) -> Result<(), String> {
    validate_file_path(&p.file)?;
    validate_apply_mode(&p.apply_mode)?;
    validate_value_type(&p.value_type)?;

    if p.label.trim().is_empty() {
        return Err(tr("label cannot be empty"));
    }

    if (p.apply_mode == "toml_key" || p.apply_mode == "toml_absent") && p.path.trim().is_empty() {
        return Err(trf(
            "{mode} mode requires a path",
            &[("mode", p.apply_mode.clone())],
        ));
    }
    if p.apply_mode == "toml_key" && p.value_type == "none" {
        return Err(tr("value_type of toml_key mode cannot be none"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_custom_id_adds_prefix() {
        assert_eq!(normalize_custom_id("foo"), "custom.foo");
        assert_eq!(normalize_custom_id("custom.bar"), "custom.bar");
    }

    #[test]
    fn validate_apply_mode_accepts_four_modes() {
        for m in [
            "toml_key",
            "toml_absent",
            "file_overwrite",
            "markdown_block",
        ] {
            assert!(validate_apply_mode(m).is_ok(), "{} should be valid", m);
        }
        assert!(validate_apply_mode("nonsense").is_err());
    }

    #[test]
    fn validate_param_fields_checks_toml_key_requirements() {
        let mut p = GuardParam {
            id: "custom.test".into(),
            label: "测试".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: "config.toml".into(),
            apply_mode: "toml_key".into(),
            path: String::new(),
            value_type: "bool".into(),
            default: serde_json::json!(true),
            default_en: serde_json::Value::Null,
            custom: true,
        };
        // 空 path 应该报错
        assert!(validate_param_fields(&p).is_err());
        p.path = "x.y".into();
        assert!(validate_param_fields(&p).is_ok());
        // toml_key 不能是 none 类型
        p.value_type = "none".into();
        assert!(validate_param_fields(&p).is_err());
    }

    #[test]
    fn validate_param_fields_rejects_bad_file_path() {
        let p = GuardParam {
            id: "custom.test".into(),
            label: "测试".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: "../evil.toml".into(),
            apply_mode: "file_overwrite".into(),
            path: String::new(),
            value_type: "text".into(),
            default: serde_json::json!("hi"),
            default_en: serde_json::Value::Null,
            custom: true,
        };
        assert!(validate_param_fields(&p).is_err());
    }

    #[test]
    fn validate_file_path_rejects_traversal() {
        assert!(validate_file_path("config.toml").is_ok());
        assert!(validate_file_path("agents/foo.toml").is_ok());
        assert!(validate_file_path("../escape").is_err());
        assert!(validate_file_path("a/../b").is_err());
        assert!(validate_file_path("/absolute").is_err());
        assert!(validate_file_path("").is_err());
    }

    #[test]
    fn validate_param_for_file_accepts_matching_modes() {
        let mut p = GuardParam {
            id: "p".into(),
            label: "参数".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: "config.toml".into(),
            apply_mode: "toml_key".into(),
            path: "features.x".into(),
            value_type: "bool".into(),
            default: serde_json::json!(true),
            default_en: serde_json::Value::Null,
            custom: true,
        };
        assert!(validate_param_for_file(&p, GuardFileFormat::Toml).is_ok());
        assert!(validate_param_for_file(&p, GuardFileFormat::Json).is_err());
        p.apply_mode = "file_overwrite".into();
        assert!(validate_param_for_file(&p, GuardFileFormat::PlainText).is_ok());
    }
}
