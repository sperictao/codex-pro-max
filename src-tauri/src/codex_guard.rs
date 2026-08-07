//! Codex 配置看守：schema 驱动的 ~/.codex 参数托管、锁定与漂移恢复。
//! 词汇与语义边界见仓库 CONTEXT.md 与 docs/adr/0001。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

use crate::config;
use crate::i18n::{tr, trf};

const BUILTIN_SCHEMA: &str = include_str!("guard_schema.json");

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

/// 按界面语言挑文案：en 且有英文资源时用英文，否则用原文（纯函数，便于测试）
fn pick_i18n<'a>(primary: &'a str, en: &'a str, lang: &str) -> &'a str {
    if lang == "en" && !en.is_empty() {
        en
    } else {
        primary
    }
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

fn schema_file_path() -> Result<PathBuf, String> {
    Ok(config::home_dir()?
        .join(".dashi-taskboard-launcher")
        .join("codex-guard-schema.json"))
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

const CUSTOM_ID_PREFIX: &str = "custom.";

fn normalize_custom_id(id: &str) -> String {
    if id.starts_with(CUSTOM_ID_PREFIX) {
        id.to_string()
    } else {
        format!("{}{}", CUSTOM_ID_PREFIX, id)
    }
}

fn validate_file_path(rel: &str) -> Result<(), String> {
    if rel.trim().is_empty() {
        return Err(tr("File path cannot be empty"));
    }
    if rel.starts_with('/') || rel.starts_with('\\') {
        return Err(tr("File path must be relative to ~/.codex and cannot start with /"));
    }
    for seg in rel.split(['/', '\\']) {
        if seg == ".." {
            return Err(tr("File path cannot contain .. and must stay inside ~/.codex"));
        }
    }
    Ok(())
}

fn validate_format(format: &str) -> Result<(), String> {
    match format {
        "toml" | "json" | "md" => Ok(()),
        other => Err(trf("Unsupported file format: {format}", &[("format", other.to_string())])),
    }
}

fn validate_guard_file(f: &GuardFile) -> Result<(), String> {
    if f.name.trim().is_empty() {
        return Err(tr("File name cannot be empty"));
    }
    validate_file_path(&f.file)?;
    validate_format(&f.format)?;
    Ok(())
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

/// 加载 schema：磁盘同 id 覆盖内置（用户定制内置参数），磁盘独有条目保留；
/// 例外：label_en/description_en/default_en 是随二进制发布的英文资源、不算用户数据，
/// 始终以内置为准——否则旧版本释放到磁盘的中文副本会永久滞留（i18n 实机踩坑）
pub fn load_schema() -> Vec<GuardParam> {
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

// ============ TOML 路径读写（toml_edit 保留注释与格式） ============

fn get_toml_path<'a>(doc: &'a DocumentMut, path: &str) -> Option<&'a Item> {
    let mut item = doc.as_item();
    for seg in path.split('.') {
        item = item.get(seg)?;
    }
    Some(item)
}

fn set_toml_path(doc: &mut DocumentMut, path: &str, val: Item) -> Result<(), String> {
    let segs: Vec<&str> = path.split('.').collect();
    let mut item = doc.as_item_mut();
    for seg in &segs[..segs.len() - 1] {
        if item.as_table_like().is_none() {
            return Err(trf("Intermediate path node {node} is not a table", &[("node", seg.to_string())]));
        }
        if item.get(seg).is_none() {
            item.as_table_like_mut()
                .unwrap()
                .insert(seg, Item::Table(Table::new()));
        }
        item = item.get_mut(seg).unwrap();
        if item.as_table_like().is_none() {
            return Err(trf("Intermediate path node {node} is not a table", &[("node", seg.to_string())]));
        }
    }
    let last = segs[segs.len() - 1];
    item.as_table_like_mut()
        .ok_or_else(|| tr("Path endpoint is not a table"))?
        .insert(last, val);
    Ok(())
}

fn remove_toml_path(doc: &mut DocumentMut, path: &str) {
    let segs: Vec<&str> = path.split('.').collect();
    let mut item = doc.as_item_mut();
    for seg in &segs[..segs.len() - 1] {
        match item.get_mut(seg) {
            Some(i) => item = i,
            None => return,
        }
    }
    if let Some(t) = item.as_table_like_mut() {
        t.remove(segs[segs.len() - 1]);
    }
}

fn json_to_toml(v: &serde_json::Value) -> Result<Item, String> {
    match v {
        serde_json::Value::Bool(b) => Ok(toml_edit::value(*b)),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(toml_edit::value)
            .ok_or_else(|| tr("Integer parameters do not support decimals")),
        serde_json::Value::String(s) => Ok(toml_edit::value(s.as_str())),
        _ => Err(tr("Unsupported value type")),
    }
}

fn toml_matches_json(item: &Item, v: &serde_json::Value) -> bool {
    match (item, v) {
        (Item::Value(toml_edit::Value::Boolean(b)), serde_json::Value::Bool(x)) => {
            *b.value() == *x
        }
        (Item::Value(toml_edit::Value::Integer(i)), serde_json::Value::Number(n)) => {
            n.as_i64() == Some(*i.value())
        }
        (Item::Value(toml_edit::Value::String(s)), serde_json::Value::String(x)) => {
            s.value() == x
        }
        _ => false,
    }
}

fn render_toml_value(item: &Item) -> String {
    match item {
        Item::Value(toml_edit::Value::Boolean(b)) => b.value().to_string(),
        Item::Value(toml_edit::Value::Integer(i)) => i.value().to_string(),
        Item::Value(toml_edit::Value::String(s)) => format!("\"{}\"", s.value()),
        other => other.to_string(),
    }
}

// ============ markdown_block ============

fn block_begin(param: &GuardParam) -> String {
    format!("<!-- dashi:begin {} -->", param.id)
}

fn block_end(param: &GuardParam) -> String {
    format!("<!-- dashi:end {} -->", param.id)
}

fn extract_block<'a>(content: &'a str, begin: &str, end: &str) -> Option<&'a str> {
    let b = content.find(begin)?;
    let after = &content[b + begin.len()..];
    let e = after.find(end)?;
    Some(after[..e].trim())
}

fn upsert_block(content: &str, begin: &str, end: &str, block_content: &str) -> String {
    let block = format!("{}\n{}\n{}", begin, block_content.trim(), end);
    if let (Some(b), Some(e_start)) = (content.find(begin), content.find(end)) {
        if b <= e_start {
            let e = e_start + end.len();
            return format!("{}{}{}", &content[..b], block, &content[e..]);
        }
    }
    if content.trim().is_empty() {
        format!("{}\n", block)
    } else {
        format!("{}\n\n{}\n", content.trim_end(), block)
    }
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
            match extract_block(&content, &block_begin(param), &block_end(param)) {
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
                &block_begin(param),
                &block_end(param),
                expected.as_str().unwrap_or(""),
            );
            write_with_backup(&param.file, &file, &new_content)
        }
        other => Err(trf("Unknown apply_mode: {mode}", &[("mode", other.to_string())])),
    }
}

/// 按界面语言挑 default 值：en 且有英文内容时用英文，否则用原文（纯函数，便于测试）
fn default_for_lang<'a>(p: &'a GuardParam, lang: &str) -> &'a serde_json::Value {
    if lang == "en" && !p.default_en.is_null() {
        &p.default_en
    } else {
        &p.default
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

fn validate_apply_mode(mode: &str) -> Result<(), String> {
    match mode {
        "toml_key" | "toml_absent" | "file_overwrite" | "markdown_block" => Ok(()),
        other => Err(trf("Unsupported apply_mode: {mode}", &[("mode", other.to_string())])),
    }
}

fn validate_value_type(value_type: &str) -> Result<(), String> {
    match value_type {
        "bool" | "int" | "string" | "text" | "none" => Ok(()),
        other => Err(trf("Unsupported value_type: {type}", &[("type", other.to_string())])),
    }
}

fn validate_param_fields(p: &GuardParam) -> Result<(), String> {
    validate_file_path(&p.file)?;
    validate_apply_mode(&p.apply_mode)?;
    validate_value_type(&p.value_type)?;

    if p.label.trim().is_empty() {
        return Err(tr("label cannot be empty"));
    }

    if (p.apply_mode == "toml_key" || p.apply_mode == "toml_absent") && p.path.trim().is_empty() {
        return Err(trf("{mode} mode requires a path", &[("mode", p.apply_mode.clone())]));
    }
    if p.apply_mode == "toml_key" && p.value_type == "none" {
        return Err(tr("value_type of toml_key mode cannot be none"));
    }

    Ok(())
}

/// 加载磁盘上的用户 schema（不含内置合并）
fn load_disk_schema() -> Result<Vec<GuardParam>, String> {
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

fn save_disk_schema(schema: &[GuardParam]) -> Result<(), String> {
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
    fn toml_set_get_remove_roundtrip() {
        let mut doc = "[mcp_servers.x]\nurl = \"u\"\n"
            .parse::<DocumentMut>()
            .unwrap();
        set_toml_path(
            &mut doc,
            "features.multi_agent_v2.enabled",
            json_to_toml(&serde_json::json!(true)).unwrap(),
        )
        .unwrap();
        let item = get_toml_path(&doc, "features.multi_agent_v2.enabled").unwrap();
        assert!(toml_matches_json(item, &serde_json::json!(true)));
        // 既有内容未被破坏
        assert!(get_toml_path(&doc, "mcp_servers.x.url").is_some());
        // 注释保留（不紧贴被删表）
        let mut doc2 = "# 注释\nx = 1\n[agents]\nmax_threads = 6\n"
            .parse::<DocumentMut>()
            .unwrap();
        remove_toml_path(&mut doc2, "agents");
        assert!(get_toml_path(&doc2, "agents").is_none());
        assert!(doc2.to_string().contains("# 注释"));
        assert!(get_toml_path(&doc2, "x").is_some());
    }

    #[test]
    fn toml_remove_nested() {
        let mut doc = "[a.b]\nx = 1\n"
            .parse::<DocumentMut>()
            .unwrap();
        remove_toml_path(&mut doc, "a.b");
        assert!(get_toml_path(&doc, "a.b").is_none());
    }

    #[test]
    fn markdown_block_append_and_replace() {
        let s = upsert_block("# 我的笔记\n\n已有内容。\n", "<!-- b -->", "<!-- e -->", "你好");
        assert!(s.contains("已有内容。"));
        assert!(s.contains("<!-- b -->\n你好\n<!-- e -->"));
        let s2 = upsert_block(&s, "<!-- b -->", "<!-- e -->", "世界");
        assert!(s2.contains("世界"));
        assert!(!s2.contains("你好"));
        assert_eq!(extract_block(&s2, "<!-- b -->", "<!-- e -->"), Some("世界"));
    }

    // --- 自定义参数校验 ---

    #[test]
    fn normalize_custom_id_adds_prefix() {
        assert_eq!(normalize_custom_id("foo"), "custom.foo");
        assert_eq!(normalize_custom_id("custom.bar"), "custom.bar");
    }

    #[test]
    fn validate_apply_mode_accepts_four_modes() {
        for m in ["toml_key", "toml_absent", "file_overwrite", "markdown_block"] {
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
    fn builtin_schema_has_no_custom_flag() {
        let builtin: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        for p in &builtin {
            assert!(!p.custom, "内置参数 {} 不应有 custom=true", p.id);
        }
    }

    // --- 文件管理 ---

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
    fn validate_format_accepts_three() {
        for f in ["toml", "json", "md"] {
            assert!(validate_format(f).is_ok());
        }
        assert!(validate_format("yaml").is_err());
        assert!(validate_format("").is_err());
    }

}
