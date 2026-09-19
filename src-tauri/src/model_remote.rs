//! 供应商连通性验证与远端模型列表拉取。
//!
//! 两件事共用一套请求构造：按供应商的 `base_url` / `env_key` /
//! `experimental_bearer_token` / `http_headers` / `env_http_headers` /
//! `query_params` 拼出 `GET {base}/models`，读回 OpenAI 兼容的
//! `{"data":[{"id":...}]}` 列表。
//!
//! 连通性验证**不抛错**：网络失败、401、超时都是正常结果，UI 要展示的是
//! 「哪一步没通」，而不是一个红 toast。联网命令一律 async（tauri 的 tokio
//! 运行时），不占同步 IPC 通道。

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::model_credentials;
use crate::model_schema::{ProviderSchema, BUILTIN_PROVIDER};

/// 单次探测超时
const PROBE_TIMEOUT_SECS: u64 = 10;
/// 响应体读取上限：模型列表是几百 KB 级，超过即视为异常响应
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

/// 连通性验证结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectionResult {
    pub ok: bool,
    /// 真正请求的 URL（含补出的 `/models`）
    pub endpoint: String,
    /// 是否带上了鉴权头
    pub authenticated: bool,
    /// HTTP 状态码；请求未发出时为 None
    pub status: Option<u16>,
    /// 拉到的模型数；响应体不含列表时为 None
    pub model_count: Option<usize>,
    /// 失败原因（已本地化）
    pub error: Option<String>,
}

/// 远端模型条目
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteModel {
    pub id: String,
}

#[derive(Deserialize)]
struct ModelsEnvelope {
    /// 缺 `data` 键 = 不是 OpenAI 兼容列表（与"空列表"区分开）
    data: Option<Vec<RemoteModelEntry>>,
}

#[derive(Deserialize)]
struct RemoteModelEntry {
    #[serde(default)]
    id: String,
}

/// 把 base_url 补成 `${base}/models`；已以 `/models` 结尾就不再追加
pub(crate) fn models_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/models")
    }
}

/// 组装鉴权头；返回 `(头列表, 是否带上了凭据)`
fn auth_headers(p: &ProviderSchema) -> (Vec<(String, String)>, bool) {
    // 直填密钥优先：用户显式写在 config.toml 里的意图最强
    if let Some(token) = p
        .bearer_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return (
            vec![("Authorization".to_string(), format!("Bearer {token}"))],
            true,
        );
    }
    if let Some(name) = p.env_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((value, _)) = model_credentials::resolve(name) {
            return (
                vec![("Authorization".to_string(), format!("Bearer {value}"))],
                true,
            );
        }
    }
    (Vec::new(), false)
}

/// 构造请求：统一带上静态头与 env 头（env 头缺失时整条跳过，与 Codex 一致）
fn build_request(
    client: &reqwest::Client,
    p: &ProviderSchema,
    endpoint: &str,
) -> (reqwest::RequestBuilder, bool) {
    let (auth, authenticated) = auth_headers(p);
    let mut req = client.get(endpoint);
    for (k, v) in auth {
        req = req.header(k, v);
    }
    for (k, v) in &p.http_headers {
        req = req.header(k, v);
    }
    for (header, var) in &p.env_http_headers {
        if let Some((value, _)) = model_credentials::resolve(var) {
            req = req.header(header, value);
        }
    }
    if !p.query_params.is_empty() {
        let pairs: Vec<(&str, &str)> = p
            .query_params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        req = req.query(&pairs);
    }
    (req, authenticated)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .user_agent(concat!("codex-pro-max/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| {
            crate::logging::error("模型探测: 构建 HTTP 客户端", &e.to_string());
            crate::i18n::trf(
                "Failed to initialize network client: {error}",
                &[("error", e.to_string())],
            )
        })
}

/// 从响应体里解析模型 id 列表；不是合法列表（缺 `data` 键或非 JSON）返回 None。
/// 端点存在但不是 OpenAI 兼容时，UI 需要区分"连不通"与"格式不对"。
pub(crate) fn parse_model_list(body: &str) -> Option<Vec<RemoteModel>> {
    let env: ModelsEnvelope = serde_json::from_str(body).ok()?;
    let mut models: Vec<RemoteModel> = env
        .data?
        .into_iter()
        .filter(|m| !m.id.trim().is_empty())
        .map(|m| RemoteModel { id: m.id })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|a, b| a.id == b.id);
    Some(models)
}

/// 校验供应商可用于联网探测：路由键与 base_url 必需
fn validate_target(p: &ProviderSchema) -> Result<String, String> {
    let base = p.base_url.as_deref().unwrap_or("").trim();
    if base.is_empty() {
        // 内置 openai 允许留空 base_url（Codex 自带默认端点），其余必须显式声明
        if p.route == BUILTIN_PROVIDER {
            return Err(crate::i18n::tr(
                "The built-in OpenAI provider has no explicit base URL; add one to test connectivity",
            ));
        }
        return Err(crate::i18n::tr("Base URL is required to test connectivity"));
    }
    crate::model_schema::validate_base_url(base)?;
    Ok(models_endpoint(base))
}

/// 连通性验证：永远返回结果结构，网络错误也进 `error` 而不是抛给调用方
#[tauri::command]
pub async fn model_test_connection(provider: ProviderSchema) -> ConnectionResult {
    let endpoint = match validate_target(&provider) {
        Ok(e) => e,
        Err(err) => {
            return ConnectionResult {
                ok: false,
                endpoint: provider.base_url.clone().unwrap_or_default(),
                authenticated: false,
                status: None,
                model_count: None,
                error: Some(err),
            }
        }
    };

    let client = match client() {
        Ok(c) => c,
        Err(err) => {
            return ConnectionResult {
                ok: false,
                endpoint,
                authenticated: false,
                status: None,
                model_count: None,
                error: Some(err),
            }
        }
    };

    let (req, authenticated) = build_request(&client, &provider, &endpoint);
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            let ok = status.is_success();
            let count = if ok {
                resp.text().await.ok().and_then(|b| {
                    if b.len() > MAX_BODY_BYTES {
                        return None;
                    }
                    parse_model_list(&b).map(|m| m.len())
                })
            } else {
                None
            };
            ConnectionResult {
                ok,
                endpoint,
                authenticated,
                status: Some(status.as_u16()),
                model_count: count,
                error: if ok {
                    None
                } else {
                    Some(crate::i18n::trf(
                        "Endpoint returned HTTP {status}",
                        &[("status", status.as_u16().to_string())],
                    ))
                },
            }
        }
        Err(e) => {
            crate::logging::warn("模型探测: 请求失败", &e.to_string());
            ConnectionResult {
                ok: false,
                endpoint,
                authenticated,
                status: None,
                model_count: None,
                error: Some(crate::i18n::trf(
                    "Request failed: {error}",
                    &[("error", e.to_string())],
                )),
            }
        }
    }
}

/// 拉取远端模型列表（连通性验证的严格版本：失败即抛错）
#[tauri::command]
pub async fn model_remote_list(provider: ProviderSchema) -> Result<Vec<RemoteModel>, String> {
    let endpoint = validate_target(&provider)?;
    let client = client()?;
    let (req, _) = build_request(&client, &provider, &endpoint);
    let resp = req.send().await.map_err(|e| {
        crate::logging::warn("远端模型列表: 请求失败", &e.to_string());
        crate::i18n::trf(
            "Request failed: {error}",
            &[("error", e.to_string())],
        )
    })?;
    let status = resp.status();
    if !status.is_success() {
        return Err(crate::i18n::trf(
            "Endpoint returned HTTP {status}",
            &[("status", status.as_u16().to_string())],
        ));
    }
    let body = resp.text().await.map_err(|e| {
        crate::i18n::trf(
            "Failed to read response: {error}",
            &[("error", e.to_string())],
        )
    })?;
    parse_model_list(&body).ok_or_else(|| {
        crate::i18n::tr(
            "Response is not an OpenAI-compatible model list (expected a data array)",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn provider(base: Option<&str>) -> ProviderSchema {
        ProviderSchema {
            route: "test".to_string(),
            base_url: base.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn models_endpoint_appends_once() {
        assert_eq!(
            models_endpoint("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            models_endpoint("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/models"
        );
        // 已指向 /models 不再重复追加
        assert_eq!(
            models_endpoint("https://x.dev/v1/models"),
            "https://x.dev/v1/models"
        );
        assert_eq!(models_endpoint("  https://x.dev/v1  "), "https://x.dev/v1/models");
    }

    #[test]
    fn validate_target_requires_base_url_except_builtin() {
        assert!(validate_target(&provider(None)).is_err());
        assert!(validate_target(&provider(Some(""))).is_err());
        assert!(validate_target(&provider(Some("api.deepseek.com"))).is_err());
        assert_eq!(
            validate_target(&provider(Some("https://api.deepseek.com/v1"))).unwrap(),
            "https://api.deepseek.com/v1/models"
        );
        // 内置 openai 允许无 base_url，但此时无法探测（要明确告知原因）
        let builtin = ProviderSchema {
            route: BUILTIN_PROVIDER.to_string(),
            ..Default::default()
        };
        let err = validate_target(&builtin).unwrap_err();
        assert!(err.contains("built-in"), "{err}");
    }

    #[test]
    fn parse_model_list_reads_sorts_and_dedupes() {
        let body = r#"{"object":"list","data":[{"id":"b"},{"id":"a"},{"id":"a"},{"id":""},{"noid":true}]}"#;
        let models = parse_model_list(body).unwrap();
        assert_eq!(
            models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn parse_model_list_rejects_non_openai_shapes() {
        assert!(parse_model_list("not json").is_none());
        assert!(parse_model_list(r#"{"models":[{"id":"a"}]}"#).is_none());
        // 合法 JSON 但无 data：视为空列表而非错误
        assert_eq!(parse_model_list(r#"{"data":[]}"#).unwrap().len(), 0);
    }

    #[test]
    fn auth_headers_prefer_literal_token_over_env() {
        let mut p = provider(Some("https://x.dev/v1"));
        p.bearer_token = Some("sk-literal".to_string());
        p.env_key = Some("CODEX_PRO_MAX_DEFINITELY_MISSING_VAR".to_string());
        let (headers, authenticated) = auth_headers(&p);
        assert!(authenticated);
        assert_eq!(headers[0].1, "Bearer sk-literal");
    }

    #[test]
    fn auth_headers_resolve_env_and_report_absence() {
        let mut p = provider(Some("https://x.dev/v1"));
        p.env_key = Some("CODEX_PRO_MAX_DEFINITELY_MISSING_VAR".to_string());
        let (headers, authenticated) = auth_headers(&p);
        assert!(headers.is_empty());
        assert!(!authenticated, "变量缺失时不算带鉴权");

        // 用本进程必然存在的变量（HOME）验证解析路径
        let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        p.env_key = Some(name.to_string());
        let (headers, authenticated) = auth_headers(&p);
        assert!(authenticated);
        assert!(headers[0].1.starts_with("Bearer "));
    }

    #[test]
    fn build_request_skips_missing_env_headers_and_keeps_static_ones() {
        let client = reqwest::Client::new();
        let mut p = provider(Some("https://x.dev/v1"));
        p.http_headers = BTreeMap::from([("X-Static".to_string(), "v".to_string())]);
        p.env_http_headers = BTreeMap::from([
            ("X-Missing".to_string(), "CODEX_PRO_MAX_DEFINITELY_MISSING_VAR".to_string()),
            ("X-Present".to_string(), (if cfg!(windows) { "USERPROFILE" } else { "HOME" }).to_string()),
        ]);
        let (req, _) = build_request(&client, &p, "https://x.dev/v1/models");
        let built = req.build().unwrap();
        let headers = built.headers();
        assert_eq!(headers.get("X-Static").unwrap(), "v");
        assert!(headers.get("X-Present").is_some());
        assert_eq!(headers.get("X-Missing"), None, "变量缺失的头不应发出");
    }

    #[test]
    fn build_request_applies_query_params() {
        let client = reqwest::Client::new();
        let mut p = provider(Some("https://x.openai.azure.com/openai"));
        p.query_params = BTreeMap::from([(
            "api-version".to_string(),
            "2025-04-01-preview".to_string(),
        )]);
        let (req, _) = build_request(&client, &p, "https://x.openai.azure.com/openai/models");
        let built = req.build().unwrap();
        assert!(
            built.url().query().unwrap().contains("api-version=2025-04-01-preview"),
            "{}",
            built.url()
        );
    }
}
