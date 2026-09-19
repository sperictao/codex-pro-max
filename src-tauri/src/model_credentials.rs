//! 模型凭据：环境变量引用的解析、来源判定与用户级写入。
//!
//! Codex 只认环境变量名（`env_key`）；密钥本身从不进 config.toml（除用户
//! 自行使用 `experimental_bearer_token` 直填）。因此"凭据是否就绪"= 这个
//! 变量在 Codex 进程能看到的某一层里有非空值。
//!
//! 分层与可写性：
//! 1. 启动器**进程环境** —— Codex 作为子进程继承；只读（本应用改不了父进程环境）
//! 2. `~/.codex/.env` —— 用户级，本应用可写，是唯一能被 UI 修复的一层
//! 3. 项目 `.env`（cwd）—— 只读
//!
//! `.env` 的解析规则与 Codex 自身一致：`KEY=VALUE`，忽略空行与 `#` 注释，
//! 值两侧的单双引号剥掉。变量名大小写敏感。

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::model_schema::{CredentialInfo, CredentialSource};

/// 用户级 env 文件名（落在 `~/.codex/` 下）
const USER_ENV_FILE: &str = ".env";
/// 项目级 env 文件名（落在当前工作目录）
const PROJECT_ENV_FILE: &str = ".env";

pub(crate) fn user_env_path() -> Result<PathBuf, String> {
    crate::codex_fs::codex_file(USER_ENV_FILE)
}

pub(crate) fn project_env_path() -> Option<PathBuf> {
    std::env::current_dir().ok().map(|d| d.join(PROJECT_ENV_FILE))
}

/// 拆一行 `.env` 为 `(键, 去空白后的原始值)`；空行/注释/无 `=`/空键返回 None。
/// [`parse_env_file`] 与 [`set_user_env`] 共用这一套行识别规则。
fn split_env_line(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let stripped = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim();
    let (key, raw) = stripped.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some((key, raw.trim()))
}

/// 解析 env 文件内容为键值表；文件不存在/不可读返回空表。
/// 语法与 Codex 一致：`export ` 前缀忽略，值两侧引号剥掉，空值算已声明。
pub(crate) fn parse_env_file(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in content.lines() {
        let Some((key, raw)) = split_env_line(line) else {
            continue;
        };
        let value = match raw.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            Some(v) => v,
            None => match raw.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
                Some(v) => v,
                None => raw,
            },
        };
        out.insert(key.to_string(), value.to_string());
    }
    out
}

fn read_env_file(path: &std::path::Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .map(|c| parse_env_file(&c))
        .unwrap_or_default()
}

/// 按 Codex 实际可见性解析一个变量：进程环境 → 用户 `.env` → 项目 `.env`。
/// 返回 `(值, 来源)`；都没有就是 None。
pub(crate) fn resolve(name: &str) -> Option<(String, CredentialSource)> {
    if let Ok(v) = std::env::var(name) {
        if !v.trim().is_empty() {
            return Some((v, CredentialSource::Process));
        }
    }
    if let Ok(path) = user_env_path() {
        if let Some(v) = read_env_file(&path).get(name) {
            if !v.trim().is_empty() {
                return Some((v.clone(), CredentialSource::UserEnv));
            }
        }
    }
    if let Some(path) = project_env_path() {
        if let Some(v) = read_env_file(&path).get(name) {
            if !v.trim().is_empty() {
                return Some((v.clone(), CredentialSource::ProjectEnv));
            }
        }
    }
    None
}

/// 一个变量对 UI 的可见状态：是否就绪、来源、能否由本应用写入
pub(crate) fn describe(name: &str) -> CredentialInfo {
    match resolve(name) {
        Some((_, source)) => CredentialInfo {
            configured: true,
            source: Some(source),
            // 只有用户级 .env 是本应用能改的层
            writable: source == CredentialSource::UserEnv,
        },
        None => CredentialInfo {
            configured: false,
            source: None,
            writable: true,
        },
    }
}

/// 批量查询（保序：入参顺序即结果顺序）
pub(crate) fn describe_many(names: &[String]) -> Vec<(String, CredentialInfo)> {
    names
        .iter()
        .map(|n| (n.clone(), describe(n)))
        .collect()
}

/// 写入/清除用户级 `~/.codex/.env` 中的一个变量。
///
/// 保留文件里其他行与注释：只替换目标行，不存在则追加；`value = None`
/// 删除该行。文件写权限与 config.toml 同级（0600），因为是密钥载体。
pub(crate) fn set_user_env(name: &str, value: Option<&str>) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(crate::i18n::tr("Environment variable name cannot be empty"));
    }
    let path = user_env_path()?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let mut lines: Vec<String> = Vec::new();
    let mut replaced = false;
    for line in existing.lines() {
        if split_env_line(line).map(|(k, _)| k) == Some(name) {
            if let Some(v) = value {
                lines.push(format!("{name}={v}"));
            }
            replaced = true;
            continue;
        }
        lines.push(line.to_string());
    }
    if !replaced {
        if let Some(v) = value {
            lines.push(format!("{name}={v}"));
        }
    }

    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            crate::logging::error("凭据: 创建 .codex 目录", &e.to_string());
            crate::i18n::trf(
                "Failed to create directory: {error}",
                &[("error", e.to_string())],
            )
        })?;
    }
    std::fs::write(&path, body).map_err(|e| {
        crate::logging::error("凭据: 写入 .env", &e.to_string());
        crate::i18n::trf(
            "Failed to write {path}: {error}",
            &[
                ("path", path.display().to_string()),
                ("error", e.to_string()),
            ],
        )
    })?;
    restrict_permissions(&path);
    Ok(())
}

/// 尽力把凭据文件收紧到 0600（unix）；失败不影响写入结果
fn restrict_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// 从 config.toml 的供应商表里取出所有 `env_key` 引用（去重、排序）
pub(crate) fn collect_env_keys(providers: &[crate::model_schema::ProviderSchema]) -> Vec<String> {
    let mut names: Vec<String> = providers
        .iter()
        .filter_map(|p| p.env_key.clone())
        .filter(|s| !s.trim().is_empty())
        .collect();
    // env_http_headers 的值也是环境变量名，一并纳入可查询范围
    for p in providers {
        names.extend(
            p.env_http_headers
                .values()
                .filter(|s| !s.trim().is_empty())
                .cloned(),
        );
    }
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_file_handles_comments_quotes_and_export() {
        let parsed = parse_env_file(
            "# 注释\n\
             \n\
             PLAIN=value\n\
             export EXPORTED=v2\n\
             QUOTED=\"has spaces\"\n\
             SINGLE='single'\n\
             EMPTY=\n\
             MALFORMED_LINE\n\
             SPACED = padded \n",
        );
        assert_eq!(parsed.get("PLAIN").unwrap(), "value");
        assert_eq!(parsed.get("EXPORTED").unwrap(), "v2");
        assert_eq!(parsed.get("QUOTED").unwrap(), "has spaces");
        assert_eq!(parsed.get("SINGLE").unwrap(), "single");
        assert_eq!(parsed.get("EMPTY").unwrap(), "");
        assert_eq!(parsed.get("SPACED").unwrap(), "padded");
        assert!(!parsed.contains_key("MALFORMED_LINE"));
        assert!(!parsed.contains_key("# 注释"));
    }

    #[test]
    fn parse_env_file_keeps_equals_in_value() {
        let parsed = parse_env_file("TOKEN=abc=def=ghi\n");
        assert_eq!(parsed.get("TOKEN").unwrap(), "abc=def=ghi");
    }

    #[test]
    fn collect_env_keys_dedupes_and_includes_header_vars() {
        use crate::model_schema::ProviderSchema;
        let mk = |env: Option<&str>, hdr: Option<(&str, &str)>| ProviderSchema {
            env_key: env.map(str::to_string),
            env_http_headers: hdr
                .map(|(k, v)| BTreeMap::from([(k.to_string(), v.to_string())]))
                .unwrap_or_default(),
            ..Default::default()
        };
        let providers = vec![
            mk(Some("DEEPSEEK_API_KEY"), None),
            mk(Some("DEEPSEEK_API_KEY"), Some(("X-Trace", "TRACE_VAR"))),
            mk(None, None),
            mk(Some("  "), None),
        ];
        assert_eq!(
            collect_env_keys(&providers),
            vec![
                "DEEPSEEK_API_KEY".to_string(),
                "TRACE_VAR".to_string()
            ]
        );
    }

    #[test]
    fn describe_reports_process_env_as_read_only() {
        // 本进程环境里一定存在的变量：HOME（unix）/ USERPROFILE（windows）
        let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let info = describe(name);
        assert!(info.configured);
        assert_eq!(info.source, Some(CredentialSource::Process));
        assert!(!info.writable, "进程环境不由本应用写入");
    }

    #[test]
    fn describe_reports_missing_as_writable() {
        let info = describe("CODEX_PRO_MAX_DEFINITELY_MISSING_VAR");
        assert!(!info.configured);
        assert_eq!(info.source, None);
        assert!(info.writable, "未配置时用户级 .env 仍是可写入路径");
    }

    #[test]
    fn describe_many_preserves_order() {
        let names = vec!["A_MISSING_ONE".to_string(), "B_MISSING_TWO".to_string()];
        let out = describe_many(&names);
        assert_eq!(out[0].0, "A_MISSING_ONE");
        assert_eq!(out[1].0, "B_MISSING_TWO");
    }

    #[test]
    fn set_user_env_rejects_empty_name() {
        assert!(set_user_env("  ", Some("v")).is_err());
    }
}
