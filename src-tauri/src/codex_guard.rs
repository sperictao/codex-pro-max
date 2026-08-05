//! Codex 配置看守：schema 驱动的 ~/.codex 参数托管、锁定与漂移恢复。
//! 词汇与语义边界见仓库 CONTEXT.md 与 docs/adr/0001。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table};

use crate::config;

const BUILTIN_SCHEMA: &str = include_str!("guard_schema.json");

// ============ schema 与状态类型 ============

/// schema 中的一条托管参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardParam {
    pub id: String,
    pub group: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub name: String,
    pub file: String,
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

/// 加载 schema：内置覆盖同 id 磁盘条目，磁盘独有条目保留（无 version，见 CONTEXT.md）
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

    let mut merged = builtin;
    for d in disk {
        if let Some(slot) = merged.iter_mut().find(|m| m.id == d.id) {
            *slot = d; // 磁盘同 id 覆盖（用户定制内置参数）
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
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败: {}", e))?;
    let flat = rel_file.replace(['/', '\\'], "_");
    let dest = dir.join(format!("{}.{}.bak", flat, now_secs()));
    std::fs::copy(target, &dest).map_err(|e| format!("备份失败: {}", e))?;

    // 只保留 20 份：文件名即时间戳，字典序可排
    let mut olds: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("读取备份目录失败: {}", e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map_or(false, |n| n.starts_with(&format!("{}.", flat)) && n.ends_with(".bak"))
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
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    std::fs::write(target, content).map_err(|e| format!("写入 {} 失败: {}", target.display(), e))
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
            return Err(format!("路径中间节点 {} 不是表", seg));
        }
        if item.get(seg).is_none() {
            item.as_table_like_mut()
                .unwrap()
                .insert(seg, Item::Table(Table::new()));
        }
        item = item.get_mut(seg).unwrap();
        if item.as_table_like().is_none() {
            return Err(format!("路径中间节点 {} 不是表", seg));
        }
    }
    let last = segs[segs.len() - 1];
    item.as_table_like_mut()
        .ok_or_else(|| "路径终点不是表".to_string())?
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
            .ok_or_else(|| "整数参数不支持小数".to_string()),
        serde_json::Value::String(s) => Ok(toml_edit::value(s.as_str())),
        _ => Err("不支持的值类型".to_string()),
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
        Err(e) => return err(format!("读取失败: {}", e)),
    };

    match param.apply_mode.as_str() {
        "toml_key" => {
            let content = match content {
                None => return ok("missing", Some("(文件不存在)".to_string())),
                Some(c) => c,
            };
            let doc = match content.parse::<DocumentMut>() {
                Ok(d) => d,
                Err(e) => return err(format!("TOML 解析失败，已暂停该组看守: {}", e)),
            };
            match get_toml_path(&doc, &param.path) {
                None => ok("missing", Some("(未设置)".to_string())),
                Some(item) if toml_matches_json(item, expected) => {
                    ok("match", Some(render_toml_value(item)))
                }
                Some(item) => ok("drift", Some(render_toml_value(item))),
            }
        }
        "toml_absent" => {
            let content = match content {
                None => return ok("match", Some("不存在".to_string())),
                Some(c) => c,
            };
            let doc = match content.parse::<DocumentMut>() {
                Ok(d) => d,
                Err(e) => return err(format!("TOML 解析失败，已暂停该组看守: {}", e)),
            };
            if get_toml_path(&doc, &param.path).is_some() {
                ok("drift", Some("存在".to_string()))
            } else {
                ok("match", Some("不存在".to_string()))
            }
        }
        "file_overwrite" => match content {
            None => ok("missing", Some("(文件不存在)".to_string())),
            Some(c) if c.trim() == expected.as_str().unwrap_or("").trim() => {
                ok("match", Some(format!("{} 字节", c.len())))
            }
            Some(c) => ok("drift", Some(format!("{} 字节，内容不一致", c.len()))),
        },
        "markdown_block" => {
            let content = match content {
                None => return ok("missing", Some("(文件不存在)".to_string())),
                Some(c) => c,
            };
            match extract_block(&content, &block_begin(param), &block_end(param)) {
                None => ok("missing", Some("(托管区块不存在)".to_string())),
                Some(b) if b == expected.as_str().unwrap_or("").trim() => {
                    ok("match", Some("区块一致".to_string()))
                }
                Some(_) => ok("drift", Some("区块内容不一致".to_string())),
            }
        }
        other => err(format!("未知 apply_mode: {}", other)),
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
                .map_err(|e| format!("TOML 解析失败，未写入: {}", e))?;
            set_toml_path(&mut doc, &param.path, json_to_toml(expected)?)?;
            write_with_backup(&param.file, &file, &doc.to_string())
        }
        "toml_absent" => {
            let content = match std::fs::read_to_string(&file) {
                Ok(c) => c,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(format!("读取失败: {}", e)),
            };
            let mut doc = content
                .parse::<DocumentMut>()
                .map_err(|e| format!("TOML 解析失败，未写入: {}", e))?;
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
        other => Err(format!("未知 apply_mode: {}", other)),
    }
}

fn expected_of(param: &GuardParam, state: Option<&GuardParamState>) -> serde_json::Value {
    state
        .and_then(|s| s.value.clone())
        .unwrap_or_else(|| param.default.clone())
}

// ============ 视图组装 ============

pub fn build_view() -> Result<GuardView, String> {
    let cfg = config::load_config().unwrap_or_default();
    let schema = load_schema();

    let mut groups: Vec<GroupView> = Vec::new();
    for p in &schema {
        let state = cfg.codex_guard.params.get(&p.id);
        let expected = expected_of(p, state);
        let c = check(p, &expected);
        let view = ParamView {
            id: p.id.clone(),
            label: p.label.clone(),
            description: p.description.clone(),
            apply_mode: p.apply_mode.clone(),
            value_type: p.value_type.clone(),
            path: p.path.clone(),
            default: p.default.clone(),
            value: expected,
            applied: state.map_or(false, |s| s.applied),
            locked: state.map_or(false, |s| s.locked),
            actual: c.actual,
            status: c.status,
            error: c.error,
            last_checked: state.and_then(|s| s.last_checked),
            last_restored: state.and_then(|s| s.last_restored),
        };
        if let Some(g) = groups.iter_mut().find(|g| g.name == p.group) {
            if g.error.is_none() {
                g.error = view.error.clone();
            }
            g.params.push(view);
        } else {
            groups.push(GroupView {
                name: p.group.clone(),
                file: p.file.clone(),
                error: view.error.clone(),
                params: vec![view],
            });
        }
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
    let mut dirty = false;
    for p in &schema {
        let locked = cfg
            .codex_guard
            .params
            .get(&p.id)
            .map_or(false, |s| s.locked);
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
        .ok_or_else(|| format!("schema 中不存在参数: {}", id))
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
        other => return Err(format!("参数类型 {} 不可编辑", other)),
    };
    if !type_ok {
        return Err("值类型不匹配".to_string());
    }
    let mut cfg = config::load_config()?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    if st.locked {
        return Err("参数已锁定，先解锁再修改".to_string());
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
            return Err("请先启用该参数再锁定".to_string());
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
}
