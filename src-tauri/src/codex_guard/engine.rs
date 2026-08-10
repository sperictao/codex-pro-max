//! 引擎：比对期望状态与实际状态（check），把期望值写入 codex 文件（apply，写入前备份）。
//! 所有物理文件先经过统一格式校验；校验失败时绝不重写文件。

use crate::i18n::{tr, trf};

use super::backup::write_with_backup;
use super::format::{diagnostics_message, parse_toml_document, validate_bytes};
use super::markdown_block::{block_begin, block_end, extract_block, upsert_block};
use super::model::{GuardFileFormat, ValidationDiagnostic};
use super::ownership::validate_target_path;
use super::schema::default_for_lang;
use super::toml_ops::{
    get_toml_path, json_to_toml, remove_toml_path, render_toml_value, set_toml_path,
    toml_matches_json,
};
use super::validate::validate_param_for_file;
use super::{AppPaths, GuardParam, GuardParamState};

pub struct CheckResult {
    pub status: String, // match | drift | missing | error
    pub actual: Option<String>,
    pub error: Option<String>,
}

fn ok(status: &str, actual: Option<String>) -> CheckResult {
    CheckResult {
        status: status.to_string(),
        actual,
        error: None,
    }
}

fn err(msg: String) -> CheckResult {
    CheckResult {
        status: "error".to_string(),
        actual: None,
        error: Some(msg),
    }
}

/// 比对某参数的期望状态与实际状态。格式解析失败只报结构化摘要，绝不重写文件。
pub(crate) fn check(
    paths: &AppPaths,
    param: &GuardParam,
    format: GuardFileFormat,
    expected: &serde_json::Value,
) -> CheckResult {
    if let Err(error) = validate_mode_format(param, format) {
        return err(error);
    }
    let relative_file = match validate_target_path(paths, &param.file) {
        Ok(relative_file) => relative_file,
        Err(error) => return err(error.to_string()),
    };
    let file = paths.codex_file(&relative_file);
    let content = match read_existing(&file) {
        Ok(Some(content)) => content,
        Ok(None) => return missing_result(param),
        Err(error) => return err(error),
    };
    if let Err(diagnostics) = validate_bytes(format, &content, &param.id, Some(&param.file)) {
        return err(format_validation_error(&diagnostics));
    }

    match param.apply_mode.as_str() {
        "toml_key" => {
            let doc = match parse_toml_document(&content, &param.id, Some(&param.file)) {
                Ok(doc) => doc,
                Err(diagnostics) => return err(format_validation_error(&diagnostics)),
            };
            match get_toml_path(&doc, &param.path) {
                None => ok("missing", Some(tr("(not set)"))),
                Some(item) if toml_matches_json(item, expected) => {
                    ok("match", Some(render_toml_value(item)))
                }
                Some(item) => ok("drift", Some(render_toml_value(item))),
            }
        }
        "toml_absent" => {
            let doc = match parse_toml_document(&content, &param.id, Some(&param.file)) {
                Ok(doc) => doc,
                Err(diagnostics) => return err(format_validation_error(&diagnostics)),
            };
            if get_toml_path(&doc, &param.path).is_some() {
                ok("drift", Some(tr("present")))
            } else {
                ok("match", Some(tr("absent")))
            }
        }
        "file_overwrite" => {
            let expected = match expected_text(expected) {
                Ok(value) => value,
                Err(error) => return err(error),
            };
            let content = match String::from_utf8(content) {
                Ok(content) => content,
                Err(_) => return err(tr("Guard file is not valid UTF-8")),
            };
            if content.trim() == expected.trim() {
                ok(
                    "match",
                    Some(trf("{n} bytes", &[("n", content.len().to_string())])),
                )
            } else {
                ok(
                    "drift",
                    Some(trf(
                        "{n} bytes, content differs",
                        &[("n", content.len().to_string())],
                    )),
                )
            }
        }
        "markdown_block" => {
            let content = match String::from_utf8(content) {
                Ok(content) => content,
                Err(_) => return err(tr("Guard file is not valid UTF-8")),
            };
            let expected = match expected_text(expected) {
                Ok(value) => value,
                Err(error) => return err(error),
            };
            match extract_block(&content, &block_begin(&param.id), &block_end(&param.id)) {
                None => ok("missing", Some(tr("(managed block does not exist)"))),
                Some(block) if block == expected.trim() => ok("match", Some(tr("block matches"))),
                Some(_) => ok("drift", Some(tr("block content differs"))),
            }
        }
        other => err(trf(
            "Unknown apply_mode: {mode}",
            &[("mode", other.to_string())],
        )),
    }
}

/// 把期望值写入 codex 文件（写入前备份）。
pub(crate) fn apply(
    paths: &AppPaths,
    param: &GuardParam,
    format: GuardFileFormat,
    expected: &serde_json::Value,
) -> Result<(), String> {
    validate_mode_format(param, format)?;
    let relative_file =
        validate_target_path(paths, &param.file).map_err(|error| error.to_string())?;
    let file = paths.codex_file(&relative_file);
    match param.apply_mode.as_str() {
        "toml_key" => {
            let content = read_existing(&file)?;
            let mut doc = parse_toml_document(
                content.as_deref().unwrap_or_default(),
                &param.id,
                Some(&param.file),
            )
            .map_err(|diagnostics| format_validation_error(&diagnostics))?;
            set_toml_path(&mut doc, &param.path, json_to_toml(expected)?)?;
            write_with_backup(paths, &param.file, &file, &doc.to_string())
        }
        "toml_absent" => {
            let content = match read_existing(&file)? {
                None => return Ok(()),
                Some(content) => content,
            };
            let mut doc = parse_toml_document(&content, &param.id, Some(&param.file))
                .map_err(|diagnostics| format_validation_error(&diagnostics))?;
            remove_toml_path(&mut doc, &param.path);
            write_with_backup(paths, &param.file, &file, &doc.to_string())
        }
        "file_overwrite" => {
            let mut candidate = expected_text(expected)?.trim().to_string();
            candidate.push('\n');
            validate_existing(&file, format, param)?;
            validate_bytes(format, candidate.as_bytes(), &param.id, Some(&param.file))
                .map_err(|diagnostics| format_validation_error(&diagnostics))?;
            write_with_backup(paths, &param.file, &file, &candidate)
        }
        "markdown_block" => {
            let content = read_existing(&file)?.unwrap_or_default();
            validate_bytes(format, &content, &param.id, Some(&param.file))
                .map_err(|diagnostics| format_validation_error(&diagnostics))?;
            let content =
                String::from_utf8(content).map_err(|_| tr("Guard file is not valid UTF-8"))?;
            let new_content = upsert_block(
                &content,
                &block_begin(&param.id),
                &block_end(&param.id),
                expected_text(expected)?,
            );
            validate_bytes(format, new_content.as_bytes(), &param.id, Some(&param.file))
                .map_err(|diagnostics| format_validation_error(&diagnostics))?;
            write_with_backup(paths, &param.file, &file, &new_content)
        }
        other => Err(trf(
            "Unknown apply_mode: {mode}",
            &[("mode", other.to_string())],
        )),
    }
}

fn read_existing(file: &std::path::Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(file) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(trf("Read failed: {error}", &[("error", error.to_string())])),
    }
}

fn validate_existing(
    file: &std::path::Path,
    format: GuardFileFormat,
    param: &GuardParam,
) -> Result<(), String> {
    let Some(content) = read_existing(file)? else {
        return Ok(());
    };
    validate_bytes(format, &content, &param.id, Some(&param.file))
        .map_err(|diagnostics| format_validation_error(&diagnostics))
}

fn validate_mode_format(param: &GuardParam, format: GuardFileFormat) -> Result<(), String> {
    validate_param_for_file(param, format)
}

fn expected_text(expected: &serde_json::Value) -> Result<&str, String> {
    expected
        .as_str()
        .ok_or_else(|| tr("Expected value must be text for this apply mode"))
}

fn format_validation_error(diagnostics: &[ValidationDiagnostic]) -> String {
    trf(
        "Guard file validation failed: {diagnostic}",
        &[("diagnostic", diagnostics_message(diagnostics))],
    )
}

fn missing_result(param: &GuardParam) -> CheckResult {
    match param.apply_mode.as_str() {
        "toml_absent" => ok("match", Some(tr("absent"))),
        "toml_key" | "file_overwrite" | "markdown_block" => {
            ok("missing", Some(tr("(file does not exist)")))
        }
        _ => err(trf(
            "Unknown apply_mode: {mode}",
            &[("mode", param.apply_mode.clone())],
        )),
    }
}

/// 期望值计算：用户改过的值永远优先；否则期望值随界面语言（带 default_en 的参数）。
pub(crate) fn expected_of(
    param: &GuardParam,
    state: Option<&GuardParamState>,
) -> serde_json::Value {
    state
        .and_then(|s| s.value.clone())
        .unwrap_or_else(|| default_for_lang(param, crate::i18n::current()).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(file: &str, apply_mode: &str, path: &str, value_type: &str) -> GuardParam {
        GuardParam {
            id: "custom.test".into(),
            label: "测试".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: file.into(),
            apply_mode: apply_mode.into(),
            path: path.into(),
            value_type: value_type.into(),
            default: serde_json::Value::Null,
            default_en: serde_json::Value::Null,
            custom: true,
        }
    }

    #[test]
    fn apply_rejects_directory_without_treating_it_as_an_empty_document() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(&target).unwrap();
        let result = apply(
            &paths,
            &param("config.toml", "toml_key", "features.enabled", "bool"),
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        );
        assert!(result.is_err());
        assert!(target.is_dir());
    }

    #[test]
    fn apply_rejects_invalid_toml_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = b"[features\nthis is invalid\n";
        std::fs::write(&target, original).unwrap();
        let result = apply(
            &paths,
            &param("config.toml", "toml_key", "features.enabled", "bool"),
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), original);
    }

    #[test]
    fn apply_allows_missing_toml_only_as_an_explicit_empty_document() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let param = param("config.toml", "toml_key", "features.enabled", "bool");
        apply(
            &paths,
            &param,
            GuardFileFormat::Toml,
            &serde_json::json!(true),
        )
        .unwrap();
        let content = std::fs::read_to_string(paths.codex_file("config.toml")).unwrap();
        let document = content.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(toml_matches_json(
            get_toml_path(&document, "features.enabled").unwrap(),
            &serde_json::json!(true),
        ));
    }

    #[test]
    fn check_reports_format_code_without_echoing_json_content() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("settings.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, br#"{"TOP_SECRET":1,"TOP_SECRET":2}"#).unwrap();
        let result = check(
            &paths,
            &param("settings.json", "file_overwrite", "", "text"),
            GuardFileFormat::Json,
            &serde_json::json!("{}"),
        );
        let error = result.error.unwrap();
        assert!(error.contains("json_duplicate_key"));
        assert!(!error.contains("TOP_SECRET"));
    }
}
