//! 模型目录：models.dev 全量快照的拉取、缓存与按已配置路由的投影。
//!
//! 目录**不是事实来源**：config.toml 才是。这里只回答两个问题——
//! 「这个模型官方标称多少上下文/输出、支持哪些模态与推理档」（供表单预填
//! 与只读展示），以及「当前配置的模型在目录里有没有对应条目」。
//!
//! 缓存落在 `~/.codex-pro-max/models-cache.json`，过期后由 UI 显式刷新；
//! 缓存缺失/损坏一律当作"没有目录"降级，不影响模型配置本身的读写。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::codex_fs::now_secs;

/// models.dev 全量目录端点（与 CCursor / dsh 同源）
pub(crate) const MODELS_DEV_API: &str = "https://models.dev/api.json";
/// 快照过期阈值（秒）：超过即提示可刷新，但不阻塞使用
pub(crate) const CATALOG_STALE_SECS: i64 = 24 * 3600;
/// 单次拉取超时
const FETCH_TIMEOUT_SECS: u64 = 30;

/// 目录条目：目录里**一个上游供应商下的一款模型**
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogEntry {
    /// models.dev 的供应商键（如 deepseek、anthropic、openai）
    pub provider: String,
    /// 模型 id（请求身份）
    pub id: String,
    /// 目录显示名；缺失回落 id
    pub name: String,
    /// 上下文窗口（token）
    #[serde(default)]
    pub context: Option<i64>,
    /// 最大输出（token）
    #[serde(default)]
    pub max_tokens: Option<i64>,
    /// 已发布的输入模态（text/image/pdf/audio/...）
    #[serde(default)]
    pub input: Vec<String>,
    /// 目录是否声明支持推理
    #[serde(default)]
    pub reasoning: bool,
    /// 规范化后的推理档（canonical order：off/minimal/low/medium/high/xhigh/max）
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
}

/// 目录快照文件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogFile {
    /// 拉取时刻（unix 秒）
    pub fetched_at: i64,
    /// 上游至少发布一款模型的供应商数
    pub provider_count: usize,
    pub entries: Vec<CatalogEntry>,
}

/// 前端目录视图：按**已配置路由**筛选后的条目 + 快照新鲜度
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogView {
    /// 缓存缺失（从未拉取或缓存损坏）时为 true，UI 显示引导而非空表
    pub missing: bool,
    /// 拉取时刻（unix 秒）
    pub fetched_at: i64,
    /// 是否已过 [`CATALOG_STALE_SECS`]
    pub stale: bool,
    /// 该路由在目录里能找到的条目；键 = 配置里的路由键
    pub providers: BTreeMap<String, Vec<CatalogEntry>>,
}

// ============ models.dev 原始形态 ============

#[derive(Deserialize)]
struct RawProvider {
    #[serde(default)]
    models: BTreeMap<String, RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    limit: Option<RawLimit>,
    #[serde(default)]
    reasoning: Option<bool>,
    #[serde(default)]
    modalities: Option<RawModalities>,
}

#[derive(Deserialize)]
struct RawLimit {
    #[serde(default)]
    context: Option<i64>,
    #[serde(default)]
    output: Option<i64>,
}

#[derive(Deserialize)]
struct RawModalities {
    #[serde(default)]
    input: Vec<String>,
}

/// 目录声明支持推理但没给可用档位时的保守默认（与 PI-Desktop 规则一致）：
/// models.dev 对多数模型只发布 `reasoning: true` 而不列 values。
const DEFAULT_REASONING_LEVELS: [&str; 3] = ["low", "medium", "high"];

fn raw_levels(model: &RawModel) -> Vec<String> {
    match model.reasoning {
        None | Some(false) => Vec::new(),
        Some(true) => DEFAULT_REASONING_LEVELS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
    }
}

/// 把 models.dev 原始 JSON 投影成扁平条目表。
/// 只保留上游显式发布的字段：缺 context 就是缺，不猜。
pub(crate) fn project_catalog(raw: &str) -> Result<CatalogFile, String> {
    let providers: BTreeMap<String, RawProvider> = serde_json::from_str(raw).map_err(|e| {
        crate::i18n::trf(
            "Failed to parse model catalog: {error}",
            &[("error", e.to_string())],
        )
    })?;

    let mut entries = Vec::new();
    let mut provider_count = 0usize;
    for (provider_key, provider) in providers {
        if provider.models.is_empty() {
            continue;
        }
        provider_count += 1;
        for (model_key, model) in provider.models {
            let id = model.id.clone().unwrap_or(model_key);
            let mut input = model
                .modalities
                .as_ref()
                .map(|m| m.input.clone())
                .unwrap_or_default();
            input.sort();
            input.dedup();
            // 目录未发布 limit 时按缺省处理，不写 0 冒充"窗口为 0"
            let (context, max_tokens) = model
                .limit
                .as_ref()
                .map(|l| (l.context, l.output))
                .unwrap_or((None, None));
            entries.push(CatalogEntry {
                provider: provider_key.clone(),
                name: model.name.clone().unwrap_or_else(|| id.clone()),
                id,
                context,
                max_tokens,
                input,
                reasoning: model.reasoning.unwrap_or(false),
                reasoning_levels: raw_levels(&model),
            });
        }
    }
    entries.sort_by(|a, b| a.provider.cmp(&b.provider).then(a.id.cmp(&b.id)));
    Ok(CatalogFile {
        fetched_at: now_secs() as i64,
        provider_count,
        entries,
    })
}

// ============ 缓存 ============

pub(crate) fn cache_path() -> Result<PathBuf, String> {
    Ok(crate::config::home_dir()?
        .join(".codex-pro-max")
        .join("models-cache.json"))
}

/// 读缓存；文件缺失或损坏一律当作"没有目录"（`Ok(None)`）。
/// 目录是增强信息，坏缓存不该让模型页报错。
pub(crate) fn load_cache() -> Option<CatalogFile> {
    let path = cache_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<CatalogFile>(&raw) {
        Ok(f) => Some(f),
        Err(e) => {
            crate::logging::warn("模型目录缓存损坏，按无目录处理", &e.to_string());
            None
        }
    }
}

/// 写缓存：目录是可选增强，写失败只记日志，不上抛
pub(crate) fn save_cache(file: &CatalogFile) {
    let Ok(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string(file) {
        Ok(body) => {
            if let Err(e) = std::fs::write(&path, body) {
                crate::logging::warn("写入模型目录缓存失败", &e.to_string());
            }
        }
        Err(e) => crate::logging::warn("序列化模型目录缓存失败", &e.to_string()),
    }
}

/// 拉取 models.dev 并写入缓存；返回落盘后的快照。
/// 异步：跑在 tauri 的 tokio 运行时上，27MB 下载不占同步 IPC 通道。
pub(crate) async fn refresh() -> Result<CatalogFile, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS))
        .user_agent(concat!("codex-pro-max/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| {
            crate::logging::error("模型目录: 构建 HTTP 客户端", &e.to_string());
            crate::i18n::trf(
                "Failed to initialize network client: {error}",
                &[("error", e.to_string())],
            )
        })?;

    let resp = client
        .get(MODELS_DEV_API)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            crate::logging::error("模型目录: 拉取 models.dev", &e.to_string());
            crate::i18n::trf(
                "Failed to fetch model catalog: {error}",
                &[("error", e.to_string())],
            )
        })?;
    let body = resp.text().await.map_err(|e| {
        crate::logging::error("模型目录: 读取响应体", &e.to_string());
        crate::i18n::trf(
            "Failed to read response: {error}",
            &[("error", e.to_string())],
        )
    })?;

    let file = project_catalog(&body)?;
    save_cache(&file);
    Ok(file)
}

// ============ 路由 → 目录供应商 ============

/// 目录供应商键的归一形式：只留小写字母数字，用于「spero-ai」↔「speroai」这类差异
fn normalize_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// 把配置里的路由键解析到目录供应商键。
///
/// 命中顺序：完全同名 → 归一后同名 → 路由键是目录键的前缀/后缀
/// （`deepseek-v3` 与 `deepseek`）。都不命中就返回 None，
/// 表示"目录不覆盖这个路由"——UI 据此只做只读提示，不误报缺模型。
fn resolve_provider<'a>(route: &str, known: &'a [String]) -> Option<&'a String> {
    if let Some(hit) = known.iter().find(|k| k.as_str() == route) {
        return Some(hit);
    }
    let norm = normalize_key(route);
    if norm.is_empty() {
        return None;
    }
    if let Some(hit) = known.iter().find(|k| normalize_key(k) == norm) {
        return Some(hit);
    }
    known
        .iter()
        .find(|k| normalize_key(k).starts_with(&norm) || norm.starts_with(&normalize_key(k)))
}

/// 按已配置的路由键筛出目录条目
pub(crate) fn view_for_routes(file: &CatalogFile, routes: &[String]) -> CatalogView {
    let known: Vec<String> = {
        let mut v: Vec<String> = file.entries.iter().map(|e| e.provider.clone()).collect();
        v.sort();
        v.dedup();
        v
    };
    let mut providers: BTreeMap<String, Vec<CatalogEntry>> = BTreeMap::new();
    for route in routes {
        let Some(hit) = resolve_provider(route, &known) else {
            continue;
        };
        let entries: Vec<CatalogEntry> = file
            .entries
            .iter()
            .filter(|e| &e.provider == hit)
            .cloned()
            .collect();
        if !entries.is_empty() {
            providers.insert(route.clone(), entries);
        }
    }
    CatalogView {
        missing: false,
        fetched_at: file.fetched_at,
        stale: now_secs() as i64 - file.fetched_at > CATALOG_STALE_SECS,
        providers,
    }
}

/// 目录缺失时的视图（从未拉取 / 缓存损坏）
pub(crate) fn missing_view() -> CatalogView {
    CatalogView {
        missing: true,
        fetched_at: 0,
        stale: true,
        providers: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "deepseek": {
        "id": "deepseek",
        "models": {
          "deepseek-chat": {
            "id": "deepseek-chat",
            "name": "DeepSeek Chat",
            "limit": { "context": 128000, "output": 8192 },
            "modalities": { "input": ["text"] },
            "reasoning": false
          },
          "deepseek-reasoner": {
            "id": "deepseek-reasoner",
            "name": "DeepSeek Reasoner",
            "limit": { "context": 128000, "output": 65536 },
            "modalities": { "input": ["text"] },
            "reasoning": true
          }
        }
      },
      "anthropic": {
        "models": {
          "claude-sonnet-4": {
            "name": "Claude Sonnet 4",
            "limit": { "context": 200000, "output": 64000 },
            "modalities": { "input": ["text", "image"] },
            "reasoning": true
          }
        }
      },
      "empty-provider": { "models": {} }
    }"#;

    #[test]
    fn project_flattens_and_skips_empty_providers() {
        let file = project_catalog(SAMPLE).unwrap();
        // empty-provider 不计数、不产条目
        assert_eq!(file.provider_count, 2);
        assert_eq!(file.entries.len(), 3);
        let chat = file
            .entries
            .iter()
            .find(|e| e.id == "deepseek-chat")
            .unwrap();
        assert_eq!(chat.provider, "deepseek");
        assert_eq!(chat.name, "DeepSeek Chat");
        assert_eq!(chat.context, Some(128_000));
        assert_eq!(chat.max_tokens, Some(8192));
        assert_eq!(chat.input, vec!["text".to_string()]);
        assert!(!chat.reasoning);
        assert!(chat.reasoning_levels.is_empty());
    }

    #[test]
    fn project_falls_back_to_key_and_ordered_levels() {
        let file = project_catalog(SAMPLE).unwrap();
        let sonnet = file
            .entries
            .iter()
            .find(|e| e.id == "claude-sonnet-4")
            .unwrap();
        // 缺 id 时回落 map 键；缺 name 时回落 id
        assert_eq!(sonnet.name, "Claude Sonnet 4");
        assert_eq!(sonnet.input, vec!["image".to_string(), "text".to_string()]);
        // reasoning=true 而无档位声明 → 保守默认三档
        assert_eq!(
            sonnet.reasoning_levels,
            vec!["low".to_string(), "medium".into(), "high".into()]
        );
    }

    #[test]
    fn project_reports_missing_limits_as_none_not_zero() {
        let raw = r#"{"p":{"models":{"m":{"id":"m"}}}}"#;
        let file = project_catalog(raw).unwrap();
        let m = &file.entries[0];
        assert_eq!(m.context, None);
        assert_eq!(m.max_tokens, None);
        assert_eq!(m.name, "m");
        assert!(m.input.is_empty());
        assert!(!m.reasoning);
    }

    #[test]
    fn project_rejects_malformed_json() {
        assert!(project_catalog("not json").is_err());
        assert!(project_catalog("[]").is_err());
    }

    #[test]
    fn resolve_provider_matches_exact_then_normalized_then_prefix() {
        let known = vec![
            "deepseek".to_string(),
            "spero-ai".to_string(),
            "openai".to_string(),
        ];
        assert_eq!(resolve_provider("deepseek", &known).unwrap(), "deepseek");
        // 归一：大小写与连字符差异
        assert_eq!(resolve_provider("SperoAI", &known).unwrap(), "spero-ai");
        // 前缀：路由带后缀仍能找到目录供应商
        assert_eq!(resolve_provider("deepseek-v3", &known).unwrap(), "deepseek");
        // 目录不覆盖的路由 → None（不误报缺模型）
        assert!(resolve_provider("my-local-llama", &known).is_none());
    }

    #[test]
    fn view_filters_to_configured_routes() {
        let file = project_catalog(SAMPLE).unwrap();
        let view = view_for_routes(&file, &["deepseek".into(), "unknown-route".into()]);
        assert!(!view.missing);
        assert_eq!(view.providers.len(), 1, "目录不覆盖的路由不出现");
        assert_eq!(view.providers["deepseek"].len(), 2);
        assert!(!view.stale, "刚投影的快照不过期");
    }

    #[test]
    fn view_marks_stale_snapshot() {
        let mut file = project_catalog(SAMPLE).unwrap();
        file.fetched_at = now_secs() as i64 - CATALOG_STALE_SECS - 10;
        let view = view_for_routes(&file, &["deepseek".into()]);
        assert!(view.stale);
    }

    #[test]
    fn missing_view_is_explicit() {
        let view = missing_view();
        assert!(view.missing);
        assert!(view.stale);
        assert!(view.providers.is_empty());
    }

    #[test]
    fn normalize_key_strips_punctuation_and_case() {
        assert_eq!(normalize_key("spero-ai"), "speroai");
        assert_eq!(normalize_key("Spero_AI"), "speroai");
        assert_eq!(normalize_key("---"), "");
    }
}
