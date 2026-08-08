//! Codex 配置看守：schema 驱动的 ~/.codex 参数托管、锁定与漂移恢复。
//! 词汇与语义边界见仓库 CONTEXT.md 与 docs/adr/0001。

mod markdown_block;
mod schema;
mod toml_ops;
mod validate;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml_edit::DocumentMut;

use crate::config;
use crate::i18n::{tr, trf};

use markdown_block::{block_begin, block_end, extract_block, upsert_block};
use schema::{
    default_for_lang, load_disk_schema, load_schema, pick_i18n, save_disk_schema, schema_file_path,
};
use toml_ops::{
    get_toml_path, json_to_toml, remove_toml_path, render_toml_value, set_toml_path,
    toml_matches_json,
};
use validate::{normalize_custom_id, validate_file_path, validate_guard_file, validate_param_fields};

// ============ schema 与状态类型 ============

/// schema 中的一条托管参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardParam {
    pub id: String,
    pub label: String,
    /// 英文 label；空 = 无英文资源，落回 label 原文（自定义参数即如此）
    #[serde(default)]
    pub label_en: String,
    #[serde(default)]
    pub description: String,
    /// 英文 description；空 = 落回 description 原文
    #[serde(default)]
    pub description_en: String,
    /// 相对 ~/.codex 的路径，如 config.toml / AGENTS.md / agents/default.toml
    pub file: String,
    /// toml_key | toml_absent | file_overwrite | markdown_block
    pub apply_mode: String,
    /// 点分 TOML 路径（toml 模式用）
    #[serde(default)]
    pub path: String,
    /// bool | int | string | text | none
    #[serde(default)]
    pub value_type: String,
    #[serde(default)]
    pub default: serde_json::Value,
    /// 英文 default（写入内容本身双语时用）；Null = 内容不随界面语言变化
    #[serde(default)]
    pub default_en: serde_json::Value,
    /// 是否为用户自定义参数（非内置）；自定义参数可删除
    #[serde(default)]
    pub custom: bool,
}

impl GuardParam {
    /// 当前界面语言下的 label（default 值是写入 codex 的内容，不在翻译范围）
    pub fn localized_label(&self) -> &str {
        pick_i18n(&self.label, &self.label_en, crate::i18n::current())
    }

    pub fn localized_description(&self) -> &str {
        pick_i18n(&self.description, &self.description_en, crate::i18n::current())
    }
}

/// 单个参数的托管状态，持久化在 LauncherConfig.codex_guard.params
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardParamState {
    /// 用户改后的值；None = 用 schema 推荐值
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub applied: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub last_checked: Option<u64>,
    #[serde(default)]
    pub last_restored: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexGuardState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub params: HashMap<String, GuardParamState>,
    /// 看守目标文件列表（内置 + 自定义）
    #[serde(default)]
    pub files: Vec<GuardFile>,
}

/// 看守目标文件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardFile {
    pub id: String,
    pub name: String,
    /// 相对 ~/.codex 的路径
    pub file: String,
    /// toml | json | md
    pub format: String,
    #[serde(default)]
    pub builtin: bool,
    /// 上次路径检测记录；None = 从未检测（检测走文件系统，落盘后不再重复扫）
    #[serde(default)]
    pub detection: Option<DetectRecord>,
}

/// 一次路径检测的结果记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectRecord {
    /// 检测到的相对 ~/.codex 路径；None = 未找到该文件
    pub path: Option<String>,
    pub at: u64,
}

// ============ 给前端的视图 ============

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub apply_mode: String,
    pub value_type: String,
    pub path: String,
    pub default: serde_json::Value,
    pub value: serde_json::Value,
    pub applied: bool,
    pub locked: bool,
    pub actual: Option<String>,
    /// match | drift | missing | error
    pub status: String,
    pub error: Option<String>,
    pub last_checked: Option<u64>,
    pub last_restored: Option<u64>,
    pub custom: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub file: String,
    pub format: String,
    pub builtin: bool,
    pub error: Option<String>,
    pub params: Vec<ParamView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardView {
    pub enabled: bool,
    pub groups: Vec<GroupView>,
}

// ============ 路径与基础工具 ============

fn codex_home() -> Result<PathBuf, String> {
    Ok(config::home_dir()?.join(".codex"))
}

fn codex_file(rel: &str) -> Result<PathBuf, String> {
    Ok(codex_home()?.join(rel))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ============ 文件管理 ============

fn builtin_files() -> Vec<GuardFile> {
    vec![
        GuardFile {
            id: "builtin.config-toml".to_string(),
            name: "config.toml".to_string(),
            file: "config.toml".to_string(),
            format: "toml".to_string(),
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.agents-md".to_string(),
            name: "AGENTS.md".to_string(),
            file: "AGENTS.md".to_string(),
            format: "md".to_string(),
            builtin: true,
            detection: None,
        },
        GuardFile {
            id: "builtin.default-toml".to_string(),
            name: "default.toml".to_string(),
            file: "agents/default.toml".to_string(),
            format: "toml".to_string(),
            builtin: true,
            detection: None,
        },
    ]
}

/// 加载文件列表；若配置为空则初始化内置文件并持久化
pub fn load_files() -> Result<Vec<GuardFile>, String> {
    let mut cfg = config::load_config()?;
    if cfg.codex_guard.files.is_empty() {
        cfg.codex_guard.files = builtin_files();
        config::save_config(&cfg)?;
    }
    Ok(cfg.codex_guard.files.clone())
}

fn save_files(files: &[GuardFile]) -> Result<(), String> {
    let mut cfg = config::load_config()?;
    cfg.codex_guard.files = files.to_vec();
    config::save_config(&cfg)
}

fn find_file(files: &[GuardFile], id: &str) -> Option<GuardFile> {
    files.iter().find(|f| f.id == id).cloned()
}

// ============ 备份 ============

/// 写入前备份目标文件到 ~/.codex/dashi-backups/，每个文件保留 20 份
fn backup(rel_file: &str, target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Ok(());
    }
    let dir = codex_home()?.join("dashi-backups");
    std::fs::create_dir_all(&dir).map_err(|e| trf("Failed to create backup directory: {error}", &[("error", e.to_string())]))?;
    let flat = rel_file.replace(['/', '\\'], "_");
    let dest = dir.join(format!("{}.{}.bak", flat, now_secs()));
    std::fs::copy(target, &dest).map_err(|e| trf("Backup failed: {error}", &[("error", e.to_string())]))?;

    // 只保留 20 份：文件名即时间戳，字典序可排
    let mut olds: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| trf("Failed to read backup directory: {error}", &[("error", e.to_string())]))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&format!("{}.", flat)) && n.ends_with(".bak"))
        })
        .collect();
    olds.sort();
    while olds.len() > 20 {
        let _ = std::fs::remove_file(olds.remove(0));
    }
    Ok(())
}

fn write_with_backup(rel_file: &str, target: &Path, content: &str) -> Result<(), String> {
    backup(rel_file, target)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| trf("Failed to create directory: {error}", &[("error", e.to_string())]))?;
    }
    std::fs::write(target, content).map_err(|e| trf("Failed to write {path}: {error}", &[
        ("path", target.display().to_string()),
        ("error", e.to_string()),
    ]))
}

// ============ 检查与写入 ============

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
pub fn check(param: &GuardParam, expected: &serde_json::Value) -> CheckResult {
    let file = match codex_file(&param.file) {
        Ok(f) => f,
        Err(e) => return err(e),
    };
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
pub fn apply(param: &GuardParam, expected: &serde_json::Value) -> Result<(), String> {
    let file = codex_file(&param.file)?;
    match param.apply_mode.as_str() {
        "toml_key" => {
            let content = std::fs::read_to_string(&file).unwrap_or_default();
            let mut doc = content
                .parse::<DocumentMut>()
                .map_err(|e| trf("TOML parse failed; nothing written: {error}", &[("error", e.to_string())]))?;
            set_toml_path(&mut doc, &param.path, json_to_toml(expected)?)?;
            write_with_backup(&param.file, &file, &doc.to_string())
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
            write_with_backup(&param.file, &file, &doc.to_string())
        }
        "file_overwrite" => {
            let mut content = expected.as_str().unwrap_or("").trim().to_string();
            content.push('\n');
            write_with_backup(&param.file, &file, &content)
        }
        "markdown_block" => {
            let content = std::fs::read_to_string(&file).unwrap_or_default();
            let new_content = upsert_block(
                &content,
                &block_begin(&param.id),
                &block_end(&param.id),
                expected.as_str().unwrap_or(""),
            );
            write_with_backup(&param.file, &file, &new_content)
        }
        other => Err(trf("Unknown apply_mode: {mode}", &[("mode", other.to_string())])),
    }
}

fn expected_of(param: &GuardParam, state: Option<&GuardParamState>) -> serde_json::Value {
    // 用户改过的值永远优先；否则期望值随界面语言（带 default_en 的参数）
    state
        .and_then(|s| s.value.clone())
        .unwrap_or_else(|| default_for_lang(param, crate::i18n::current()).clone())
}

// ============ 视图组装 ============

pub fn build_view() -> Result<GuardView, String> {
    let cfg = config::load_config().unwrap_or_default();
    let schema = load_schema();
    let files = load_files().unwrap_or_else(|_| builtin_files());

    let mut groups: Vec<GroupView> = Vec::new();
    for f in &files {
        let mut group_params: Vec<ParamView> = Vec::new();
        let mut group_error: Option<String> = None;

        for p in schema.iter().filter(|p| p.file == f.file) {
            let state = cfg.codex_guard.params.get(&p.id);
            let expected = expected_of(p, state);
            let c = check(p, &expected);
            if group_error.is_none() && c.error.is_some() {
                group_error = c.error.clone();
            }
            group_params.push(ParamView {
                id: p.id.clone(),
                label: p.localized_label().to_string(),
                description: p.localized_description().to_string(),
                apply_mode: p.apply_mode.clone(),
                value_type: p.value_type.clone(),
                path: p.path.clone(),
                default: p.default.clone(),
                value: expected,
                applied: state.is_some_and(|s| s.applied),
                locked: state.is_some_and(|s| s.locked),
                actual: c.actual,
                status: c.status,
                error: c.error,
                last_checked: state.and_then(|s| s.last_checked),
                last_restored: state.and_then(|s| s.last_restored),
                custom: p.custom,
            });
        }

        groups.push(GroupView {
            id: f.id.clone(),
            name: f.name.clone(),
            file: f.file.clone(),
            format: f.format.clone(),
            builtin: f.builtin,
            error: group_error,
            params: group_params,
        });
    }
    Ok(GuardView {
        enabled: cfg.codex_guard.enabled,
        groups,
    })
}

// ============ 轮询（60s，仅 launcher 运行期间） ============

pub async fn poll_loop() {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Err(e) = poll_once() {
            log::error!("codex guard 轮询失败: {}", e);
        }
    }
}

fn poll_once() -> Result<(), String> {
    let mut cfg = config::load_config()?;
    if !cfg.codex_guard.enabled {
        return Ok(());
    }
    let schema = load_schema();
    // 只看守文件列表内的目标文件，与 UI 可见范围一致（CONTEXT.md：UI 完全由合并结果驱动）
    let files = load_files().unwrap_or_else(|_| builtin_files());
    let mut dirty = false;
    for p in &schema {
        if !files.iter().any(|f| f.file == p.file) {
            continue;
        }
        let locked = cfg
            .codex_guard
            .params
            .get(&p.id)
            .is_some_and(|s| s.locked);
        if !locked {
            continue;
        }
        let expected = expected_of(p, cfg.codex_guard.params.get(&p.id));
        let c = check(p, &expected);
        let st = cfg.codex_guard.params.entry(p.id.clone()).or_default();
        st.last_checked = Some(now_secs());
        dirty = true;
        if c.status == "drift" || c.status == "missing" {
            match apply(p, &expected) {
                Ok(()) => {
                    st.last_restored = Some(now_secs());
                    log::info!("codex guard 已自动恢复: {}", p.id);
                }
                Err(e) => log::error!("codex guard 恢复 {} 失败: {}", p.id, e),
            }
        }
    }
    if dirty {
        config::save_config(&cfg)?;
    }
    Ok(())
}

// ============ Tauri 命令 ============

fn find_param(schema: &[GuardParam], id: &str) -> Result<GuardParam, String> {
    schema
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| trf("Parameter not found in schema: {id}", &[("id", id.to_string())]))
}

#[tauri::command]
pub fn guard_get_view() -> Result<GuardView, String> {
    build_view()
}

#[tauri::command]
pub fn guard_set_enabled(enabled: bool) -> Result<(), String> {
    let mut cfg = config::load_config()?;
    cfg.codex_guard.enabled = enabled;
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_value(id: String, value: serde_json::Value) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let type_ok = match p.value_type.as_str() {
        "bool" => value.is_boolean(),
        "int" => value.as_i64().is_some(),
        "string" | "text" => value.is_string(),
        other => return Err(trf("Parameter type {type} is not editable", &[("type", other.to_string())])),
    };
    if !type_ok {
        return Err(tr("Value type mismatch"));
    }
    let mut cfg = config::load_config()?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    if st.locked {
        return Err(tr("Parameter is locked; unlock it before modifying"));
    }
    st.value = Some(value);
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_apply(id: String) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let mut cfg = config::load_config()?;
    let expected = expected_of(&p, cfg.codex_guard.params.get(&id));
    apply(&p, &expected)?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    st.applied = true;
    st.last_checked = Some(now_secs());
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_applied(id: String, applied: bool) -> Result<(), String> {
    if applied {
        return guard_apply(id);
    }
    // 禁用只取消看守，不回滚已写入 ~/.codex/ 的值（与删除参数的语义一致）
    let mut cfg = config::load_config()?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    if st.locked {
        return Err(tr("Unlock the parameter before disabling it"));
    }
    st.applied = false;
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_locked(id: String, locked: bool) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let mut cfg = config::load_config()?;
    {
        let st = cfg.codex_guard.params.entry(id.clone()).or_default();
        if locked && !st.applied {
            return Err(tr("Apply the parameter before locking it"));
        }
        st.locked = locked;
    }
    if locked {
        // 锁定即校验一次：已漂移就当场恢复
        let expected = expected_of(&p, cfg.codex_guard.params.get(&id));
        let c = check(&p, &expected);
        let st = cfg.codex_guard.params.entry(id).or_default();
        st.last_checked = Some(now_secs());
        if c.status == "drift" || c.status == "missing" {
            apply(&p, &expected)?;
            st.last_restored = Some(now_secs());
        }
    }
    config::save_config(&cfg)
}

// ============ 自定义参数管理 ============

#[tauri::command]
pub fn guard_add_custom_param(
    mut param: GuardParam,
    file_id: String,
) -> Result<(), String> {
    let files = load_files()?;
    let f = find_file(&files, &file_id)
        .ok_or_else(|| trf("Target file not found: {id}", &[("id", file_id.clone())]))?;

    param.id = normalize_custom_id(&param.id);
    param.custom = true;
    param.file = f.file.clone();
    validate_param_fields(&param)?;

    let mut disk = load_disk_schema().unwrap_or_default();
    if let Some(slot) = disk.iter_mut().find(|p| p.id == param.id) {
        *slot = param;
    } else {
        disk.push(param);
    }
    save_disk_schema(&disk)
}

#[tauri::command]
pub fn guard_remove_custom_param(id: String) -> Result<(), String> {
    let normalized = normalize_custom_id(&id);

    let mut disk = load_disk_schema().unwrap_or_default();
    let before = disk.len();
    disk.retain(|p| p.id != normalized);
    if disk.len() == before {
        return Err(trf("Custom parameter not found: {id}", &[("id", normalized.clone())]));
    }
    save_disk_schema(&disk)?;

    // 同时清理配置里的状态，但保留已写入 codex 文件的值（不回滚，与 ADR 一致）
    let mut cfg = config::load_config()?;
    cfg.codex_guard.params.remove(&normalized);
    config::save_config(&cfg)?;

    Ok(())
}

#[tauri::command]
pub fn guard_get_schema_file_path() -> Result<String, String> {
    schema_file_path().map(|p| p.to_string_lossy().to_string())
}

// ============ 文件管理命令 ============

#[tauri::command]
pub fn guard_get_files() -> Result<Vec<GuardFile>, String> {
    load_files()
}

#[tauri::command]
pub fn guard_add_file(name: String, file: String, format: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;

    // 从 name 推导 id slug（简单处理：非字母数字替换为 -，去首尾 -，小写）
    let slug = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        return Err(tr("File name must contain at least one letter or digit"));
    }
    let id = normalize_custom_id(&slug);

    // id 与路径冲突检查（同路径会让参数在两个分组里重复显示）
    if files.iter().any(|f| f.id == id) {
        return Err(trf("A file with the same name already exists: {name}", &[("name", name.clone())]));
    }
    let trimmed_file = file.trim().to_string();
    if files.iter().any(|f| f.file == trimmed_file) {
        return Err(trf("Path already in guard list: {path}", &[("path", trimmed_file.clone())]));
    }

    let gf = GuardFile {
        id: id.clone(),
        name: name.trim().to_string(),
        file: trimmed_file,
        format,
        builtin: false,
        detection: None,
    };
    validate_guard_file(&gf)?;

    files.push(gf.clone());
    save_files(&files)?;

    Ok(gf)
}

#[tauri::command]
pub fn guard_update_file(id: String, name: String, file: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;

    let old_file = files[idx].file.clone();
    let new_file = file.trim().to_string();

    if old_file != new_file && files.iter().any(|f| f.id != id && f.file == new_file) {
        return Err(trf("Path already in guard list: {path}", &[("path", new_file.clone())]));
    }

    let f = &mut files[idx];
    f.name = name.trim().to_string();
    f.file = new_file.clone();
    if old_file != new_file {
        // 路径变了，旧检测记录作废（下次打开设置页会重新检测一次）
        f.detection = None;
    }
    validate_guard_file(f)?;

    // 如果是自定义参数的归属文件，路径变了参数的 file 也要跟着变
    // schema 中该文件路径下的自定义参数需要更新 file 字段
    if old_file != new_file {
        let mut disk = load_disk_schema().unwrap_or_default();
        let mut changed = false;
        for p in disk.iter_mut() {
            if p.custom && p.file == old_file {
                p.file = new_file.clone();
                changed = true;
            }
        }
        if changed {
            save_disk_schema(&disk)?;
        }
    }

    save_files(&files)?;
    Ok(files[idx].clone())
}

#[tauri::command]
pub fn guard_remove_file(id: String) -> Result<(), String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;

    if files[idx].builtin {
        return Err(tr("Built-in files cannot be removed"));
    }

    let target_file = files[idx].file.clone();
    files.remove(idx);
    save_files(&files)?;

    // 清理该文件下的所有自定义参数（schema + 状态）
    // 不回滚已写入 codex 的值（与 ADR 一致）
    let mut disk = load_disk_schema().unwrap_or_default();
    // 先收集待删参数的 id 再删 schema（删完就查不到了），用于清理配置里的状态
    let removed_ids: Vec<String> = disk
        .iter()
        .filter(|p| p.custom && p.file == target_file)
        .map(|p| p.id.clone())
        .collect();
    disk.retain(|p| !(p.custom && p.file == target_file));
    if !removed_ids.is_empty() {
        save_disk_schema(&disk)?;
    }

    let mut cfg = config::load_config()?;
    for pid in &removed_ids {
        cfg.codex_guard.params.remove(pid);
    }
    config::save_config(&cfg)?;

    Ok(())
}

// ============ 路径检测 ============

// ponytail: 只搜顶层 + 一层子目录；配置散得更深再升级递归
fn detect_file_path_in(home: &Path, rel: &str) -> Option<String> {
    if home.join(rel).exists() {
        return Some(rel.to_string());
    }
    let name = Path::new(rel).file_name()?.to_string_lossy().to_string();
    for e in std::fs::read_dir(home).ok()?.flatten() {
        let dir = e.path();
        if dir.is_dir() && dir.join(&name).exists() {
            return Some(format!("{}/{}", e.file_name().to_string_lossy(), name));
        }
    }
    None
}

fn detect_file_path(rel: &str) -> Option<String> {
    detect_file_path_in(&codex_home().ok()?, rel)
}

/// 检测文件实际路径并落盘记录；之后直接读记录，不重复扫盘
#[tauri::command]
pub fn guard_detect_file(id: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
    let detected = detect_file_path(&files[idx].file);
    let f = &mut files[idx];
    f.detection = Some(DetectRecord {
        path: detected,
        at: now_secs(),
    });
    let out = f.clone();
    save_files(&files)?;
    Ok(out)
}

/// 把文件选择器选中的绝对路径换算为相对 ~/.codex 的路径（越界拒绝）
#[tauri::command]
pub fn guard_relativize_picked_path(abs_path: String) -> Result<String, String> {
    let home = codex_home()?;
    let rel = Path::new(&abs_path)
        .strip_prefix(&home)
        .map_err(|_| tr("Selected file must be inside ~/.codex"))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    validate_file_path(&rel)?;
    Ok(rel)
}

// ============ 自校验 ============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_files_has_three_entries() {
        let files = builtin_files();
        assert_eq!(files.len(), 3);
        for f in &files {
            assert!(f.builtin, "{} 应该是内置文件", f.id);
            assert!(f.id.starts_with("builtin."));
            validate_guard_file(f).unwrap();
        }
    }

    #[test]
    fn detect_file_path_finds_config_and_shallow_nested() {
        let home = std::env::temp_dir().join(format!("dashi-detect-test-{}", std::process::id()));
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::write(home.join("config.toml"), "").unwrap();
        std::fs::write(home.join("agents/default.toml"), "").unwrap();

        // 原位置命中
        assert_eq!(detect_file_path_in(&home, "config.toml"), Some("config.toml".into()));
        assert_eq!(
            detect_file_path_in(&home, "agents/default.toml"),
            Some("agents/default.toml".into())
        );
        // 配置写顶层但实际在子目录 → 浅搜命中
        assert_eq!(detect_file_path_in(&home, "default.toml"), Some("agents/default.toml".into()));
        // 不存在 → None
        assert_eq!(detect_file_path_in(&home, "nope.toml"), None);

        std::fs::remove_dir_all(&home).ok();
    }
}
