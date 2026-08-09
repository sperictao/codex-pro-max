//! 引擎：比对期望状态与实际状态（check），把期望值写入 codex 文件（apply，写入前备份）。
//! TOML 解析失败只报错误，绝不重写文件。

use toml_edit::DocumentMut;

use crate::i18n::{tr, trf};

use super::backup::write_with_backup;
use super::markdown_block::{block_begin, block_end, extract_block, upsert_block};
use super::schema::default_for_lang;
use super::toml_ops::{
    get_toml_path, json_to_toml, remove_toml_path, render_toml_value, set_toml_path,
    toml_matches_json,
};
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

/// 比对某参数的期望状态与实际状态。TOML 解析失败只报错误，绝不重写文件。
pub(crate) fn check(paths: &AppPaths, param: &GuardParam, expected: &serde_json::Value) -> CheckResult {
    let file = paths.codex_file(&param.file);
    let content = match std::fs::read_to_string(&file) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return err(trf("Read failed: {error}", &[("error", e.to_string())])),
    };

    match param.apply_mode.as_str() {
        "toml_key" => {
            let content = match content {
                None => return ok("missing", Some(tr("(file does not exist)"))),
                Some(c) => c,
            };
            let doc = match content.parse::<DocumentMut>() {
                Ok(d) => d,
                Err(e) => return err(trf("TOML parse failed; guarding paused for this group: {error}", &[("error", e.to_string())])),
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
            let content = match content {
                None => return ok("match", Some(tr("absent"))),
                Some(c) => c,
            };
            let doc = match content.parse::<DocumentMut>() {
                Ok(d) => d,
                Err(e) => return err(trf("TOML parse failed; guarding paused for this group: {error}", &[("error", e.to_string())])),
            };
            if get_toml_path(&doc, &param.path).is_some() {
                ok("drift", Some(tr("present")))
            } else {
                ok("match", Some(tr("absent")))
            }
        }
        "file_overwrite" => match content {
            None => ok("missing", Some(tr("(file does not exist)"))),
            Some(c) if c.trim() == expected.as_str().unwrap_or("").trim() => {
                ok("match", Some(trf("{n} bytes", &[("n", c.len().to_string())])))
            }
            Some(c) => ok("drift", Some(trf("{n} bytes, content differs", &[("n", c.len().to_string())]))),
        },
        "markdown_block" => {
            let content = match content {
                None => return ok("missing", Some(tr("(file does not exist)"))),
                Some(c) => c,
            };
            match extract_block(&content, &block_begin(&param.id), &block_end(&param.id)) {
                None => ok("missing", Some(tr("(managed block does not exist)"))),
                Some(b) if b == expected.as_str().unwrap_or("").trim() => {
                    ok("match", Some(tr("block matches")))
                }
                Some(_) => ok("drift", Some(tr("block content differs"))),
            }
        }
        other => err(trf("Unknown apply_mode: {mode}", &[("mode", other.to_string())])),
    }
}

/// 把期望值写入 codex 文件（写入前备份）
pub(crate) fn apply(paths: &AppPaths, param: &GuardParam, expected: &serde_json::Value) -> Result<(), String> {
    let file = paths.codex_file(&param.file);
    match param.apply_mode.as_str() {
        "toml_key" => {
            let content = std::fs::read_to_string(&file).unwrap_or_default();
            let mut doc = content
                .parse::<DocumentMut>()
                .map_err(|e| trf("TOML parse failed; nothing written: {error}", &[("error", e.to_string())]))?;
            set_toml_path(&mut doc, &param.path, json_to_toml(expected)?)?;
            write_with_backup(paths, &param.file, &file, &doc.to_string())
        }
        "toml_absent" => {
            let content = match std::fs::read_to_string(&file) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(trf("Read failed: {error}", &[("error", e.to_string())])),
            };
            let mut doc = content
                .parse::<DocumentMut>()
                .map_err(|e| trf("TOML parse failed; nothing written: {error}", &[("error", e.to_string())]))?;
            remove_toml_path(&mut doc, &param.path);
            write_with_backup(paths, &param.file, &file, &doc.to_string())
        }
        "file_overwrite" => {
            let mut content = expected.as_str().unwrap_or("").trim().to_string();
            content.push('\n');
            write_with_backup(paths, &param.file, &file, &content)
        }
        "markdown_block" => {
            let content = std::fs::read_to_string(&file).unwrap_or_default();
            let new_content = upsert_block(
                &content,
                &block_begin(&param.id),
                &block_end(&param.id),
                expected.as_str().unwrap_or(""),
            );
            write_with_backup(paths, &param.file, &file, &new_content)
        }
        other => Err(trf("Unknown apply_mode: {mode}", &[("mode", other.to_string())])),
    }
}

/// 期望值计算：用户改过的值永远优先；否则期望值随界面语言（带 default_en 的参数）
pub(crate) fn expected_of(param: &GuardParam, state: Option<&GuardParamState>) -> serde_json::Value {
    state
        .and_then(|s| s.value.clone())
        .unwrap_or_else(|| default_for_lang(param, crate::i18n::current()).clone())
}
