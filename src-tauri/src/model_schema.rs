//! Codex `[model_providers.<id>]` 的 schema 层：托管字段的读写 + 非托管键原样透传。
//!
//! Codex 的供应商表在演进（`requires_openai_auth`、`supports_websockets` 等
//! 键已出现在真实 config.toml，但不在公开参考表里）。因此本模块只把**已知
//! 托管键**投影成强类型，其余键保留进 `extra` 并原样写回：编辑不丢键，
//! 未知键也不会被认成托管字段而遭静默改写。
//!
//! 事实来源 = 官方配置参考
//! （developers.openai.com/codex/config-reference）的 `model_providers.<id>.*`
//! 行。新增托管键必须同步 MANAGED_* 常量与 `docs/adr/0012-*` 的字段表。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as Json};
use toml_edit::{value, DocumentMut, InlineTable, Item, Table, Value};

/// 内置供应商 id：官方参考未列保留名单（含 ollama / lmstudio），
/// 这里只把 openai 当"删键即回落"的默认目标，其余一律按普通路由处理。
pub(crate) const BUILTIN_PROVIDER: &str = "openai";

/// 用户表里托管字段的键名（投影成结构体字段的部分）
const MANAGED_KEYS: [&str; 15] = [
    "name",
    "base_url",
    "env_key",
    "experimental_bearer_token",
    "wire_api",
    "query_params",
    "http_headers",
    "env_http_headers",
    "request_max_retries",
    "stream_max_retries",
    "stream_idle_timeout_ms",
    "startup_timeout_ms",
    "tool_timeout_sec",
    "requires_openai_auth",
    "supports_websockets",
];

/// `wire_api` 合法值；缺省 = chat（官方参考默认值）
const WIRE_APIS: [&str; 2] = ["chat", "responses"];

/// 当前模型三键可写回的值；空串统一表示「删键回落默认」
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ActiveModel {
    pub model: String,
    pub provider: String,
    pub effort: String,
}

/// 一个模型变量的取值来源；`Process` 之外都可由 UI 写入 `~/.codex/.env`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CredentialSource {
    /// 启动器进程自身环境（Codex 启动时继承）
    Process,
    /// `~/.codex/.env`（用户级，UI 可写）
    UserEnv,
    /// 项目级 `.env`（cwd 下；UI 只读）
    ProjectEnv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialInfo {
    /// 当前是否能解析出非空值
    pub configured: bool,
    /// 来源标识；未配置为 None
    pub source: Option<CredentialSource>,
    /// 只有 user-env 层可由本应用写入（其余层改了也不生效）
    pub writable: bool,
}

/// 供应商表投影：托管字段强类型 + 非托管键透传。
/// 所有托管字段都可缺省；`None` 与空集合都表示「删掉该键」。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSchema {
    /// 路由键 = providers 表的键；非空
    pub route: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "baseURL")]
    pub base_url: Option<String>,
    #[serde(default)]
    pub env_key: Option<String>,
    /// 直填密钥（对应 config.toml 的 `experimental_bearer_token`）。
    ///
    /// **只写不读**：`skip_serializing` 保证任何视图/调试输出都不会把它带出
    /// Rust；磁盘上的真值只在 `model_providers_save` 里单向写入，
    /// 存在性由 [`ProviderSchema::has_bearer_token`] 表达。
    /// 空串 = 清除该键。
    #[serde(default, skip_serializing)]
    pub bearer_token: Option<String>,
    /// chat | responses；缺省 = chat
    #[serde(default)]
    pub wire_api: Option<String>,
    #[serde(default)]
    pub query_params: BTreeMap<String, String>,
    /// 静态请求头
    #[serde(default)]
    pub http_headers: BTreeMap<String, String>,
    /// 从环境变量取值的请求头：头名 → 变量名
    #[serde(default)]
    pub env_http_headers: BTreeMap<String, String>,
    #[serde(default)]
    pub request_max_retries: Option<i64>,
    #[serde(default)]
    pub stream_max_retries: Option<i64>,
    #[serde(default)]
    pub stream_idle_timeout_ms: Option<i64>,
    /// 以下三项官方机器可读参考表未列，但真实 config.toml 已在用
    #[serde(default)]
    pub startup_timeout_ms: Option<i64>,
    #[serde(default)]
    pub tool_timeout_sec: Option<f64>,
    #[serde(default)]
    pub requires_openai_auth: Option<bool>,
    #[serde(default)]
    pub supports_websockets: Option<bool>,
    /// 非托管键原样透传（键序保留）；含上面未列出的 Codex 新键
    #[serde(default)]
    pub extra: Json,
    /// 视图元数据：表里出现过但不认识的键名（只读提示，不参与写回）
    #[serde(default)]
    pub unknown_keys: Vec<String>,
    /// 视图元数据：是否已配置直填密钥（真值不回传）
    #[serde(default)]
    pub has_bearer_token: bool,
}

/// 当前模型视图
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActiveModelView {
    pub model: String,
    pub provider: String,
    pub effort: String,
}

// ============ TOML ⇄ JSON（toml_edit ↔ serde_json，保序） ============

/// toml_edit 值 → JSON：整数保持整数，浮点保持浮点，日期时间按字符串原样投影
fn value_to_json(v: &Value) -> Json {
    match v {
        Value::String(s) => Json::String(s.value().to_string()),
        Value::Integer(i) => Json::Number((*i.value()).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f.value())
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Boolean(b) => Json::Bool(*b.value()),
        Value::Datetime(d) => Json::String(d.value().to_string()),
        Value::Array(a) => Json::Array(a.iter().map(value_to_json).collect()),
        Value::InlineTable(t) => {
            let mut map = JsonMap::new();
            for (k, val) in t.iter() {
                map.insert(k.to_string(), value_to_json(val));
            }
            Json::Object(map)
        }
    }
}

/// toml_edit Item → JSON；表递归展开
fn item_to_json(item: &Item) -> Json {
    match item {
        Item::Value(v) => value_to_json(v),
        Item::Table(t) => table_to_json(t),
        Item::ArrayOfTables(a) => Json::Array(a.iter().map(table_to_json).collect()),
        Item::None => Json::Null,
    }
}

fn table_to_json(t: &Table) -> Json {
    let mut map = JsonMap::new();
    for (k, v) in t.iter() {
        map.insert(k.to_string(), item_to_json(v));
    }
    Json::Object(map)
}

/// JSON → toml_edit Item。对象产出的永远是内联表（`{ k = v }`）：
/// `extra` 只承载标量与内联表，非托管子表（如 `[x.y]`）不在此路径上。
fn json_to_item(v: &Json) -> Item {
    match v {
        Json::Null => value(""),
        Json::Bool(b) => value(*b),
        Json::Number(n) => n
            .as_i64()
            .map(value)
            .or_else(|| n.as_f64().map(value))
            .unwrap_or_else(|| value("")),
        Json::String(s) => value(s.as_str()),
        Json::Array(a) => {
            let mut arr = toml_edit::Array::new();
            for e in a {
                if let Item::Value(val) = json_to_item(e) {
                    arr.push(val);
                }
            }
            value(arr)
        }
        Json::Object(o) => {
            let mut t = InlineTable::new();
            for (k, val) in o {
                if let Item::Value(v) = json_to_item(val) {
                    t.insert(k, v);
                }
            }
            value(t)
        }
    }
}

/// 读取表里某一层（表或内联表）的键值视图
fn table_like_get<'a>(item: &'a Item, key: &str) -> Option<&'a Item> {
    item.as_table_like()?.get(key)
}

fn str_of(item: &Item, key: &str) -> Option<String> {
    table_like_get(item, key)?
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn bool_of(item: &Item, key: &str) -> Option<bool> {
    table_like_get(item, key)?.as_bool()
}

fn int_of(item: &Item, key: &str) -> Option<i64> {
    table_like_get(item, key)?.as_integer()
}

fn float_of(item: &Item, key: &str) -> Option<f64> {
    let got = table_like_get(item, key)?;
    got.as_float().or_else(|| got.as_integer().map(|i| i as f64))
}

fn string_map_of(item: &Item, key: &str) -> BTreeMap<String, String> {
    let Some(target) = table_like_get(item, key).and_then(|i| i.as_table_like()) else {
        return BTreeMap::new();
    };
    target
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
        .collect()
}

// ============ 读取 ============

/// 从 config.toml 文档读出当前模型三键
pub(crate) fn read_active(doc: &DocumentMut) -> ActiveModel {
    let read = |key: &str| {
        doc.get(key)
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string()
    };
    ActiveModel {
        model: read("model"),
        provider: read("model_provider"),
        effort: read("model_reasoning_effort"),
    }
}

/// 把 `[model_providers]` 下的一张表投影成 `ProviderSchema`；非表条目返回 None
pub(crate) fn provider_from_table(route: &str, item: &Item) -> Option<ProviderSchema> {
    // 非托管键：标量与内联表原样带走；子表（`[model_providers.x.y]`）也带走，
    // 但写回时按"不重建子表"处理——见 write_provider 的说明。
    let table = item.as_table_like()?;
    let mut extra = JsonMap::new();
    let mut unknown_keys = Vec::new();
    for (k, v) in table.iter() {
        if MANAGED_KEYS.contains(&k) {
            continue;
        }
        extra.insert(k.to_string(), item_to_json(v));
        unknown_keys.push(k.to_string());
    }
    let bearer = str_of(item, "experimental_bearer_token").is_some();

    Some(ProviderSchema {
        route: route.to_string(),
        name: str_of(item, "name"),
        base_url: str_of(item, "base_url"),
        env_key: str_of(item, "env_key"),
        // 只留存在位标记；真值不进入视图
        bearer_token: None,
        wire_api: str_of(item, "wire_api"),
        query_params: string_map_of(item, "query_params"),
        http_headers: string_map_of(item, "http_headers"),
        env_http_headers: string_map_of(item, "env_http_headers"),
        request_max_retries: int_of(item, "request_max_retries"),
        stream_max_retries: int_of(item, "stream_max_retries"),
        stream_idle_timeout_ms: int_of(item, "stream_idle_timeout_ms"),
        startup_timeout_ms: int_of(item, "startup_timeout_ms"),
        tool_timeout_sec: float_of(item, "tool_timeout_sec"),
        requires_openai_auth: bool_of(item, "requires_openai_auth"),
        supports_websockets: bool_of(item, "supports_websockets"),
        extra: Json::Object(extra),
        unknown_keys,
        has_bearer_token: bearer,
    })
}

/// 读出 config.toml 里全部供应商（按路由键排序）
pub(crate) fn read_providers(doc: &DocumentMut) -> Vec<ProviderSchema> {
    let Some(table) = doc.get("model_providers").and_then(|i| i.as_table_like()) else {
        return Vec::new();
    };
    let mut out: Vec<ProviderSchema> = table
        .iter()
        .filter_map(|(route, item)| provider_from_table(route, item))
        .collect();
    out.sort_by(|a, b| a.route.cmp(&b.route));
    out
}

/// 供应商是否已定义（内置 openai 恒视为存在）
pub(crate) fn provider_defined(doc: &DocumentMut, route: &str) -> bool {
    route == BUILTIN_PROVIDER
        || doc
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .is_some_and(|t| t.contains_key(route))
}

/// 供应商表路径（供备份日志与错误信息使用）
pub(crate) fn provider_path(route: &str) -> String {
    format!("model_providers.{route}")
}

/// 取（必要时创建）`[model_providers.<route>]` 子表。
///
/// 父表缺失时创建；父表或子表被写成别的类型（标量/数组）时报错，不静默覆盖。
pub(crate) fn provider_table_mut<'a>(
    doc: &'a mut DocumentMut,
    route: &str,
) -> Result<&'a mut Table, String> {
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let parent = doc["model_providers"]
        .as_table_mut()
        .ok_or_else(|| crate::i18n::tr("model_providers is not a table in config.toml"))?;
    if !parent.contains_key(route) {
        parent.insert(route, Item::Table(Table::new()));
    }
    parent[route].as_table_mut().ok_or_else(|| {
        crate::i18n::trf(
            "{path} is not a table in config.toml",
            &[("path", provider_path(route))],
        )
    })
}

// ============ 写入 ============

/// 校验路由键：非空、ASCII 字母数字与 `-` `_`，且不得占用内置 id
pub(crate) fn validate_route(route: &str) -> Result<(), String> {
    if route == BUILTIN_PROVIDER {
        return Err(crate::i18n::tr(
            "openai is the built-in provider id and cannot be recreated",
        ));
    }
    let ok = !route.is_empty()
        && route
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        return Err(crate::i18n::tr(
            "Provider id may only contain letters, digits, '-' and '_'",
        ));
    }
    Ok(())
}

/// 校验 base_url：Codex 接受 http/https 端点（含本地无鉴权服务）
pub(crate) fn validate_base_url(url: &str) -> Result<(), String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        return Ok(());
    }
    Err(crate::i18n::tr("Base URL must start with http:// or https://"))
}

/// 校验 wire_api：缺省与空串都合法（缺省 = chat）
pub(crate) fn validate_wire_api(api: &str) -> Result<(), String> {
    if api.is_empty() || WIRE_APIS.contains(&api) {
        return Ok(());
    }
    Err(crate::i18n::trf(
        "Invalid wire API: {value} (expected chat or responses)",
        &[("value", api.to_string())],
    ))
}

/// 校验重试/超时数值：负数与零对 Codex 无意义，一律拒绝
pub(crate) fn validate_non_negative(label: &str, value: i64) -> Result<(), String> {
    if value >= 0 {
        return Ok(());
    }
    Err(crate::i18n::trf(
        "{label} cannot be negative",
        &[("label", label.to_string())],
    ))
}

/// 把托管字段写进（或从表中删掉）一张供应商表。
///
/// 写入语义统一为「空 = 删键」：`None`/空串/空集合都移除对应键，
/// 因此不会在 config.toml 里留下 `base_url = ""` 这类空壳。
/// 非托管键**原地保留**：`Table::insert` 对已存在的键只改值、不动位置，
/// 原有注释与排版因此不丢。
///
/// `previous_unknown_keys` 是**本次编辑前**从磁盘读到的非托管键名单
/// （即视图侧的 `unknown_keys`）。它的用途只有一个：用户在 UI 里删掉某个
/// 透传键后，磁盘上那份也要跟着消失——否则键会"删了又回来"。
/// 手写的子表（`[model_providers.x.y]`）不属于本函数的删除范围。
pub(crate) fn write_provider(
    table: &mut Table,
    p: &ProviderSchema,
    previous_unknown_keys: &[String],
) -> Result<(), String> {
    for (key, val) in [
        ("name", p.name.as_deref()),
        ("base_url", p.base_url.as_deref()),
        ("env_key", p.env_key.as_deref()),
        ("wire_api", p.wire_api.as_deref()),
    ] {
        put_str(table, key, val);
    }

    // 两个 header 表与 query_params：空表删键，不留 `http_headers = {}`
    for (key, map) in [
        ("http_headers", &p.http_headers),
        ("env_http_headers", &p.env_http_headers),
        ("query_params", &p.query_params),
    ] {
        if map.is_empty() {
            table.remove(key);
        } else {
            let mut t = InlineTable::new();
            for (k, v) in map {
                t.insert(k, Value::from(v.as_str()));
            }
            put_value(table, key, Value::InlineTable(t));
        }
    }

    for (key, val) in [
        ("request_max_retries", p.request_max_retries),
        ("stream_max_retries", p.stream_max_retries),
        ("stream_idle_timeout_ms", p.stream_idle_timeout_ms),
        ("startup_timeout_ms", p.startup_timeout_ms),
    ] {
        put_opt(table, key, val.map(value));
    }
    put_opt(table, "tool_timeout_sec", p.tool_timeout_sec.map(value));
    for (key, val) in [
        ("requires_openai_auth", p.requires_openai_auth),
        ("supports_websockets", p.supports_websockets),
    ] {
        put_opt(table, key, val.map(value));
    }

    // 透传键：先按旧名单清掉已被 UI 删除的，再写回本次仍在的。
    // 标量/内联表直接删；子表（Item::Table）跳过，保留用户手写段落。
    let live: std::collections::BTreeSet<&str> = match &p.extra {
        Json::Object(map) => map.keys().map(String::as_str).collect(),
        _ => std::collections::BTreeSet::new(),
    };
    for key in previous_unknown_keys {
        if MANAGED_KEYS.contains(&key.as_str()) || live.contains(key.as_str()) {
            continue;
        }
        if matches!(table.get(key), Some(Item::Table(_))) {
            continue;
        }
        table.remove(key);
    }
    if let Json::Object(map) = &p.extra {
        for (k, v) in map {
            if MANAGED_KEYS.contains(&k.as_str()) {
                continue;
            }
            table.insert(k, json_to_item(v));
        }
    }
    Ok(())
}

/// 写入（`Some`）或删除（`None`/空串）一个字符串键
fn put_str(table: &mut Table, key: &str, val: Option<&str>) {
    match val.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => put_value(table, key, Value::from(v)),
        None => {
            table.remove(key);
        }
    }
}

fn put_opt(table: &mut Table, key: &str, item: Option<Item>) {
    match item {
        Some(i) => put_item(table, key, i),
        None => {
            table.remove(key);
        }
    }
}

/// 就地改写既有标量：toml_edit 把「键前一行/行尾的注释」存成键值对的 decor，
/// 整条替换会连注释一起丢掉，所以类型相容时只换 value。类型不相容
/// （用户手写成数组或表）或键不存在时才整条写入。
fn put_value(table: &mut Table, key: &str, new: Value) {
    if let Some(Item::Value(slot)) = table.get_mut(key) {
        if std::mem::discriminant(slot) == std::mem::discriminant(&new) {
            *slot = new;
            return;
        }
    }
    table.insert(key, Item::Value(new));
}

/// 同 [`put_value`]，用于整数/浮点/布尔等由 `value()` 产出的整条 Item
fn put_item(table: &mut Table, key: &str, new: Item) {
    if let (Some(Item::Value(slot)), Item::Value(fresh)) = (table.get_mut(key), &new) {
        if std::mem::discriminant(slot) == std::mem::discriminant(fresh) {
            *slot = fresh.clone();
            return;
        }
    }
    table.insert(key, new);
}

/// 写入或清除 `experimental_bearer_token`。
/// `None` = 保持磁盘原值不动（UI 不回传真值，因此"没填"不能等于"清除"）；
/// `Some("")` = 用户显式清除；`Some(v)` = 写入新值。
pub(crate) fn apply_bearer_token(table: &mut Table, token: Option<&str>) {
    match token {
        None => {}
        Some(v) if v.trim().is_empty() => {
            table.remove("experimental_bearer_token");
        }
        Some(v) => put_value(table, "experimental_bearer_token", Value::from(v.trim())),
    }
}

// ============ 当前模型三键的写入 ============

/// 应用当前模型三键：空串 = 删键。预设应用走同一条路径，不设第二条写入路径。
pub(crate) fn write_active(doc: &mut DocumentMut, active: &ActiveModel) -> Result<(), String> {
    let model = active.model.trim();
    let provider = active.provider.trim();
    let effort = active.effort.trim();

    put_or_drop(doc, "model", model)?;
    // provider 为内置 openai 与空等价：都删键（codex 默认即 openai）
    if provider.is_empty() || provider == BUILTIN_PROVIDER {
        doc.remove("model_provider");
    } else {
        put_or_drop(doc, "model_provider", provider)?;
    }
    put_or_drop(doc, "model_reasoning_effort", effort)?;
    Ok(())
}

fn put_or_drop(doc: &mut DocumentMut, key: &str, val: &str) -> Result<(), String> {
    if val.is_empty() {
        doc.remove(key);
        return Ok(());
    }
    doc[key] = value(val);
    Ok(())
}

/// 删除供应商段；`[model_providers]` 空了就连表头一起删，不留空节
pub(crate) fn remove_provider(doc: &mut DocumentMut, route: &str) {
    if let Some(t) = doc
        .get_mut("model_providers")
        .and_then(|i| i.as_table_like_mut())
    {
        t.remove(route);
    }
    let empty = doc
        .get("model_providers")
        .and_then(|i| i.as_table_like())
        .is_some_and(|t| t.is_empty());
    if empty {
        doc.remove("model_providers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use toml_edit::DocumentMut;

    fn doc(src: &str) -> DocumentMut {
        src.parse::<DocumentMut>().unwrap()
    }

    fn table_of<'a>(d: &'a mut DocumentMut, route: &str) -> &'a mut Table {
        // 测试夹具：与生产走同一条取表路径
        provider_table_mut(d, route).unwrap()
    }

    #[test]
    fn read_providers_projects_managed_fields() {
        let d = doc(concat!(
            "[model_providers.azure]\n",
            "name = \"Azure\"\n",
            "base_url = \"https://x.openai.azure.com/openai\"\n",
            "env_key = \"AZURE_OPENAI_API_KEY\"\n",
            "wire_api = \"responses\"\n",
            "query_params = { api-version = \"2025-04-01-preview\" }\n",
            "http_headers = { \"X-Static\" = \"v\" }\n",
            "env_http_headers = { \"X-Env\" = \"MY_VAR\" }\n",
            "request_max_retries = 4\n",
            "stream_max_retries = 10\n",
            "stream_idle_timeout_ms = 300000\n",
        ));
        let providers = read_providers(&d);
        assert_eq!(providers.len(), 1);
        let p = &providers[0];
        assert_eq!(p.route, "azure");
        assert_eq!(p.name.as_deref(), Some("Azure"));
        assert_eq!(p.wire_api.as_deref(), Some("responses"));
        assert_eq!(p.query_params.get("api-version").unwrap(), "2025-04-01-preview");
        assert_eq!(p.http_headers.get("X-Static").unwrap(), "v");
        assert_eq!(p.env_http_headers.get("X-Env").unwrap(), "MY_VAR");
        assert_eq!(p.request_max_retries, Some(4));
        assert_eq!(p.stream_max_retries, Some(10));
        assert_eq!(p.stream_idle_timeout_ms, Some(300_000));
        assert!(p.unknown_keys.is_empty());
    }

    #[test]
    fn unknown_keys_go_to_extra_and_round_trip() {
        // 真实 config.toml 已出现公开参考表未列的键：必须原样保留
        let mut d = doc(concat!(
            "[model_providers.custom]\n",
            "name = \"OpenAI\"\n",
            "requires_openai_auth = true\n",
            "supports_websockets = true\n",
            "wire_api = \"responses\"\n",
        ));
        let p = read_providers(&d).remove(0);
        assert_eq!(p.requires_openai_auth, Some(true));
        assert_eq!(p.supports_websockets, Some(true));
        assert_eq!(p.unknown_keys.len(), 0);

        // 完全未知的新键：进 extra，并在写回后仍在
        let mut d2 = doc("[model_providers.x]\nname = \"X\"\nfuture_key = \"keep\"\n");
        let p2 = read_providers(&d2).remove(0);
        assert_eq!(p2.unknown_keys, vec!["future_key".to_string()]);
        assert_eq!(p2.extra["future_key"], Json::String("keep".into()));
        write_provider(table_of(&mut d2, "x"), &p2, &p2.unknown_keys).unwrap();
        let out = d2.to_string();
        assert!(out.contains("future_key = \"keep\""), "{out}");
        assert!(out.contains("name = \"X\""), "{out}");

        // 保留原始表，供后续断言使用
        write_provider(table_of(&mut d, "custom"), &p, &p.unknown_keys).unwrap();
        let out = d.to_string();
        assert!(out.contains("requires_openai_auth = true"), "{out}");
        assert!(out.contains("supports_websockets = true"), "{out}");
    }

    #[test]
    fn bearer_token_never_round_trips_through_view() {
        let d = doc("[model_providers.x]\nname = \"X\"\nexperimental_bearer_token = \"sk-secret\"\n");
        let p = read_providers(&d).remove(0);
        assert!(p.bearer_token.is_none(), "密钥不得回传");
        assert!(p.has_bearer_token, "只回传存在标记");
        // 序列化后的视图里不出现密钥明文
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("sk-secret"), "{json}");
        assert!(json.contains("\"hasBearerToken\":true"), "{json}");
    }

    #[test]
    fn write_provider_drops_empty_and_keeps_comments() {
        let mut d = doc(concat!(
            "# 顶部注释\n",
            "[model_providers.x]\n",
            "# 供应名称的注释\n",
            "name = \"X\"\n",
            "base_url = \"https://api.x.com/v1\"\n",
            "env_key = \"X_KEY\"\n",
        ));
        let mut p = read_providers(&d).remove(0);
        // 清空 env_key 与 header 表 → 键应被删掉而不是留空壳
        p.env_key = None;
        p.wire_api = Some("chat".to_string());
        p.http_headers.insert("A".into(), "b".into());
        write_provider(table_of(&mut d, "x"), &p, &p.unknown_keys).unwrap();
        let out = d.to_string();
        assert!(!out.contains("env_key"), "{out}");
        assert!(out.contains("wire_api = \"chat\""), "{out}");
        assert!(out.contains("http_headers = { A = \"b\" }"), "{out}");
        assert!(out.contains("# 顶部注释"), "{out}");
        // 被保留字段上的注释不因同表其他键的增删而丢
        assert!(out.contains("# 供应名称的注释"), "{out}");
    }

    #[test]
    fn write_provider_preserves_handwritten_subtable() {
        // `[model_providers.x.nested]` 是用户手写的子表，托管编辑不得抹掉
        let mut d = doc(concat!(
            "[model_providers.x]\n",
            "name = \"X\"\n",
            "\n[model_providers.x.nested]\n",
            "deep = 1\n",
        ));
        let mut p = read_providers(&d).remove(0);
        assert!(p.unknown_keys.contains(&"nested".to_string()));
        p.name = Some("X2".to_string());
        write_provider(table_of(&mut d, "x"), &p, &p.unknown_keys).unwrap();
        let out = d.to_string();
        assert!(out.contains("name = \"X2\""), "{out}");
        assert!(out.contains("deep = 1"), "{out}");
    }

    #[test]
    fn extra_keys_removed_in_view_are_removed_on_disk() {
        let mut d = doc("[model_providers.x]\nname = \"X\"\nfuture_key = \"v\"\n");
        let mut p = read_providers(&d).remove(0);
        let before = p.unknown_keys.clone();
        p.extra = Json::Object(JsonMap::new());
        write_provider(table_of(&mut d, "x"), &p, &before).unwrap();
        assert!(!d.to_string().contains("future_key"), "{}", d.to_string());
    }

    #[test]
    fn active_model_round_trip() {
        let mut d = doc("model = \"old\"\nmodel_provider = \"ds\"\ntheme = \"dark\"\n");
        write_active(
            &mut d,
            &ActiveModel {
                model: "gpt-5-codex".into(),
                provider: String::new(),
                effort: "high".into(),
            },
        )
        .unwrap();
        let a = read_active(&d);
        assert_eq!(a.model, "gpt-5-codex");
        assert_eq!(a.provider, "", "openai 与空等价 → 删键");
        assert_eq!(a.effort, "high");
        assert!(d.to_string().contains("theme = \"dark\""));

        // 三键全空 = 全删，回落 Codex 默认
        write_active(&mut d, &ActiveModel::default()).unwrap();
        let a = read_active(&d);
        assert_eq!(a, ActiveModel::default());
        assert!(!d.to_string().contains("model ="), "{}", d.to_string());
    }

    #[test]
    fn write_active_keeps_builtin_provider_key_removed() {
        let mut d = doc("model_provider = \"ds\"\n");
        write_active(
            &mut d,
            &ActiveModel {
                model: "m".into(),
                provider: BUILTIN_PROVIDER.into(),
                effort: String::new(),
            },
        )
        .unwrap();
        assert!(d.get("model_provider").is_none());
        assert_eq!(d.get("model").unwrap().as_str(), Some("m"));
    }

    #[test]
    fn remove_provider_drops_empty_table_header() {
        let mut d = doc("model_provider = \"ds\"\n\n[model_providers.ds]\nname = \"DS\"\n");
        remove_provider(&mut d, "ds");
        let out = d.to_string();
        assert!(!out.contains("[model_providers"), "{out}");
        assert!(d.get("model_provider").is_some(), "活跃键由调用方决定是否清理");
    }

    #[test]
    fn remove_provider_keeps_siblings() {
        let mut d = doc("[model_providers.ds]\nname = \"DS\"\n\n[model_providers.k]\nname = \"K\"\n");
        remove_provider(&mut d, "ds");
        assert!(!provider_defined(&d, "ds"));
        assert!(provider_defined(&d, "k"));
        assert!(provider_defined(&d, BUILTIN_PROVIDER));
    }

    #[test]
    fn provider_table_mut_creates_and_typechecks() {
        let mut d = doc("");
        provider_table_mut(&mut d, "x").unwrap();
        assert!(provider_defined(&d, "x"), "父表与子表都应被创建");

        // 父表被写成标量时报错，不静默覆盖
        let mut scalar = doc("model_providers = 3\n");
        assert!(provider_table_mut(&mut scalar, "x").is_err());
        // 子表被写成标量时报错
        let mut leaf = doc("[model_providers]\nk = 1\n");
        assert!(provider_table_mut(&mut leaf, "k").is_err());
    }

    #[test]
    fn validators_reject_bad_input() {
        assert!(validate_route("deepseek").is_ok());
        assert!(validate_route("my_provider-2").is_ok());
        assert!(validate_route("openai").is_err());
        assert!(validate_route("").is_err());
        assert!(validate_route("a.b").is_err());
        assert!(validate_route("空格").is_err());

        assert!(validate_base_url("https://api.deepseek.com/v1").is_ok());
        assert!(validate_base_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_base_url("api.deepseek.com").is_err());
        assert!(validate_base_url("").is_err());

        assert!(validate_wire_api("chat").is_ok());
        assert!(validate_wire_api("responses").is_ok());
        assert!(validate_wire_api("").is_ok());
        assert!(validate_wire_api("grpc").is_err());

        assert!(validate_non_negative("retries", 0).is_ok());
        assert!(validate_non_negative("retries", -1).is_err());
    }

    #[test]
    fn apply_bearer_token_keeps_writes_and_clears() {
        let mut d = doc("[model_providers.x]\nname = \"X\"\nexperimental_bearer_token = \"sk-old\"\n");
        // None = 不动（UI 不回传真值，未填写不得等于清除）
        apply_bearer_token(table_of(&mut d, "x"), None);
        assert!(d.to_string().contains("sk-old"), "{}", d.to_string());
        // Some(值) = 覆盖
        apply_bearer_token(table_of(&mut d, "x"), Some("sk-new"));
        assert!(d.to_string().contains("sk-new"), "{}", d.to_string());
        assert!(!d.to_string().contains("sk-old"));
        // Some("") = 显式清除
        apply_bearer_token(table_of(&mut d, "x"), Some("   "));
        assert!(!d.to_string().contains("experimental_bearer_token"), "{}", d.to_string());
    }

    #[test]
    fn json_to_item_handles_scalars_arrays_and_inline_tables() {
        let item = json_to_item(&serde_json::json!({
            "s": "v",
            "i": 3,
            "b": true,
            "arr": ["a", "b"],
            "obj": { "k": 1 }
        }));
        let inline = item.as_inline_table().expect("对象应投影成内联表");
        assert_eq!(inline.get("s").unwrap().as_str(), Some("v"));
        assert_eq!(inline.get("i").unwrap().as_integer(), Some(3));
        assert_eq!(inline.get("b").unwrap().as_bool(), Some(true));
        assert_eq!(inline.get("arr").unwrap().as_array().unwrap().len(), 2);
        assert!(inline.get("obj").unwrap().is_inline_table());
    }
}
