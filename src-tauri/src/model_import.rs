//! 模型配置导入：扫描本机其他 agent 工具已声明的供应商，导入成
//! Codex `[model_providers.*]` 段。
//!
//! 凭据语义与 Codex 一致：只认**环境变量引用**（来源里的明文密钥一律不读取、
//! 不展示、不落盘，只以计数提示用户自行补 `env_key`）。
//!
//! 扫描源缺失或损坏一律静默跳过：导入是便利功能，来源工具没装不是错误。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 单个来源条目的模型数上限与显示名截断（防病态来源文件）
const MAX_MODELS_PER_PROVIDER: usize = 64;
const MAX_NAME_LENGTH: usize = 80;

/// 导入源固定顺序 = 界面展示顺序
const SOURCES: [&str; 4] = ["codex", "claude-code", "opencode", "env"];

/// 候选的凭据形态；线格式 = `env` | `literal` | `none`，明文值永不进入本结构
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum CredentialKind {
    /// 环境变量引用
    Env,
    /// 来源持明文密钥（值不导入）
    Literal,
    /// 无凭据声明
    None,
}

/// 一个可导入候选（凭据明文永不进入本结构）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportCandidate {
    /// `source:id`，run 的选择依据
    pub key: String,
    /// 建议路由键
    pub route: String,
    /// 显示名
    pub name: String,
    #[serde(default, rename = "baseURL")]
    pub base_url: Option<String>,
    /// wire 协议：chat | responses
    #[serde(default)]
    pub wire_api: Option<String>,
    /// 环境变量引用名；仅 credential = "env" 时非空
    #[serde(default)]
    pub env_key: Option<String>,
    pub credential: CredentialKind,
    /// 该供应商在来源里声明的模型 id
    #[serde(default)]
    pub models: Vec<String>,
}

/// 按来源分组的扫描结果（固定来源顺序，缺失的来源为空组）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportGroup {
    pub source: String,
    pub entries: Vec<ImportCandidate>,
}

/// 导入执行结果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportRunResult {
    pub imported: u32,
    pub skipped: u32,
    pub failed: u32,
    /// 选中条目中来源持明文密钥的数量（提示用户补 env_key）
    pub literal: u32,
}

#[derive(Debug, Clone, PartialEq)]
enum Credential {
    /// 环境变量名引用
    Env(String),
    /// 来源持明文密钥：值不读取，仅计数
    Literal,
    None,
}

/// 扫描中间态：各来源解析出的原始草稿
#[derive(Debug, Clone)]
struct Draft {
    id: String,
    name: Option<String>,
    base_url: Option<String>,
    wire_api: Option<String>,
    credential: Credential,
    models: Vec<String>,
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// 路由键归一：来源里的 id 可能带点、空格、中文，一律压成 Codex 接受的字符集
fn sanitize_route(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_ascii_lowercase()
}

// ============ 来源：Codex 自身 ============

/// 从 `~/.codex/config.toml` 读已有供应商（换机/重装后把旧配置捡回来）
fn scan_codex(doc: &toml_edit::DocumentMut) -> Vec<Draft> {
    crate::model_schema::read_providers(doc)
        .into_iter()
        .map(|p| Draft {
            id: p.route.clone(),
            name: p.name.clone(),
            base_url: p.base_url.clone(),
            wire_api: p.wire_api.clone(),
            credential: match (p.env_key.clone(), p.has_bearer_token) {
                (Some(env), _) if !env.is_empty() => Credential::Env(env),
                (_, true) => Credential::Literal,
                _ => Credential::None,
            },
            models: Vec::new(),
        })
        .collect()
}

// ============ 来源：Claude Code ============

/// `~/.claude/settings.json` 的 env 段按 `ANTHROPIC_*` 约定声明供应商。
/// 只有存在自定义 base url 时才算一个可导入候选。
fn scan_claude_code(home: &Path) -> Vec<Draft> {
    let path = home.join(".claude").join("settings.json");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
        crate::logging::warn("导入扫描: 解析 claude settings.json 失败", "");
        return Vec::new();
    };
    let env = json.get("env").and_then(|v| v.as_object());
    let Some(env) = env else { return Vec::new() };
    let base = env
        .get("ANTHROPIC_BASE_URL")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let Some(base_url) = base.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    // 明文 token 只作存在性判定
    let credential = if env.contains_key("ANTHROPIC_AUTH_TOKEN") || env.contains_key("ANTHROPIC_API_KEY") {
        // Claude Code 不支持 env 间接引用，这里只能提示"来源持明文"
        Credential::Literal
    } else {
        Credential::None
    };
    vec![Draft {
        id: "claude-code".to_string(),
        name: Some("Claude Code".to_string()),
        base_url: Some(base_url),
        // Anthropic 原生协议与 Codex 的 chat/responses 不同构：留空让用户显式选择
        wire_api: None,
        credential,
        models: Vec::new(),
    }]
}

// ============ 来源：opencode ============

/// `~/.config/opencode/opencode.json` 的 `provider` 表
fn scan_opencode(home: &Path) -> Vec<Draft> {
    let candidates = [
        home.join(".config").join("opencode").join("opencode.json"),
        home.join(".opencode").join("opencode.json"),
    ];
    for path in candidates {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            crate::logging::warn("导入扫描: 解析 opencode.json 失败", "");
            continue;
        };
        let Some(providers) = json.get("provider").and_then(|v| v.as_object()) else {
            continue;
        };
        let mut out = Vec::new();
        for (id, entry) in providers {
            let options = entry.get("options");
            let base_url = options
                .and_then(|o| o.get("baseURL"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| truncate(s, MAX_NAME_LENGTH));
            let api_key = options
                .and_then(|o| o.get("apiKey"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // opencode 支持 `{env:VAR}` 间接引用；解出来才能映射成 env_key
            let credential = if let Some(var) = api_key
                .strip_prefix("{env:")
                .and_then(|s| s.strip_suffix('}'))
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Credential::Env(var.to_string())
            } else if api_key.trim().is_empty() {
                Credential::None
            } else {
                Credential::Literal
            };
            let models: Vec<String> = entry
                .get("models")
                .and_then(|v| v.as_object())
                .map(|m| m.keys().take(MAX_MODELS_PER_PROVIDER).cloned().collect())
                .unwrap_or_default();
            out.push(Draft {
                id: id.clone(),
                name,
                base_url,
                wire_api: None,
                credential,
                models,
            });
        }
        return out;
    }
    Vec::new()
}

// ============ 来源：环境变量 ============

/// 常见供应商的 `<NAME>_API_KEY` 约定：变量存在即提示可建同名路由
const ENV_CONVENTIONS: [(&str, &str, &str); 5] = [
    ("deepseek", "DeepSeek", "https://api.deepseek.com/v1"),
    ("moonshot", "Moonshot", "https://api.moonshot.cn/v1"),
    ("mistral", "Mistral", "https://api.mistral.ai/v1"),
    ("groq", "Groq", "https://api.groq.com/openai/v1"),
    ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
];

fn scan_env() -> Vec<Draft> {
    let mut out = Vec::new();
    for (route, label, base) in ENV_CONVENTIONS {
        let var = format!("{}_API_KEY", route.to_ascii_uppercase());
        if crate::model_credentials::resolve(&var).is_some() {
            out.push(Draft {
                id: route.to_string(),
                name: Some(label.to_string()),
                base_url: Some(base.to_string()),
                wire_api: Some("chat".to_string()),
                credential: Credential::Env(var),
                models: Vec::new(),
            });
        }
    }
    out
}

// ============ 扫描与投影 ============

/// 扫描全部来源，产出分组结果（缺失来源为空组，顺序固定）
pub(crate) fn scan() -> Vec<ImportGroup> {
    let home = crate::config::home_dir().unwrap_or_else(|_| PathBuf::from("."));

    let codex_drafts = crate::model_manifest::read_config_doc()
        .map(|doc| scan_codex(&doc))
        .unwrap_or_default();
    let sources: Vec<(&str, Vec<Draft>)> = vec![
        ("codex", codex_drafts),
        ("claude-code", scan_claude_code(&home)),
        ("opencode", scan_opencode(&home)),
        ("env", scan_env()),
    ];

    // 路由键去重：同一次扫描里后出现的同键条目标记为不可导入（前端置灰）
    let mut seen: BTreeSet<String> = BTreeSet::new();
    SOURCES
        .iter()
        .map(|want| {
            let drafts = sources
                .iter()
                .find(|(s, _)| s == want)
                .map(|(_, d)| d.clone())
                .unwrap_or_default();
            let mut entries = Vec::new();
            for d in drafts {
                let route = sanitize_route(&d.id);
                if route.is_empty() || !seen.insert(route.clone()) {
                    continue;
                }
                let (credential, env_key) = match &d.credential {
                    Credential::Env(v) => (CredentialKind::Env, Some(v.clone())),
                    Credential::Literal => (CredentialKind::Literal, None),
                    Credential::None => (CredentialKind::None, None),
                };
                entries.push(ImportCandidate {
                    key: format!("{}:{}", want, d.id),
                    route,
                    name: truncate(d.name.as_deref().unwrap_or(&d.id), MAX_NAME_LENGTH),
                    base_url: d.base_url,
                    wire_api: d.wire_api,
                    env_key,
                    credential,
                    models: d.models,
                });
            }
            ImportGroup {
                source: (*want).to_string(),
                entries,
            }
        })
        .collect()
}

/// 把选中的候选写进 config.toml。已存在的路由跳过（不覆盖用户现有配置）。
pub(crate) fn run(keys: &[String]) -> Result<ImportRunResult, String> {
    let wanted: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    let all: Vec<ImportCandidate> = scan().into_iter().flat_map(|g| g.entries).collect();
    let selected: Vec<&ImportCandidate> = all
        .iter()
        .filter(|c| wanted.contains(c.key.as_str()))
        .collect();

    let mut doc = crate::model_manifest::read_config_doc()?;
    let mut result = ImportRunResult {
        imported: 0,
        skipped: 0,
        failed: 0,
        literal: selected
            .iter()
            .filter(|c| c.credential == CredentialKind::Literal)
            .count() as u32,
    };

    for candidate in selected {
        if candidate.route == crate::model_schema::BUILTIN_PROVIDER
            || crate::model_schema::provider_defined(&doc, &candidate.route)
        {
            result.skipped += 1;
            continue;
        }
        let schema = crate::model_schema::ProviderSchema {
            route: candidate.route.clone(),
            name: Some(candidate.name.clone()),
            base_url: candidate.base_url.clone(),
            env_key: candidate.env_key.clone(),
            wire_api: candidate.wire_api.clone(),
            ..Default::default()
        };
        let table = crate::model_schema::provider_table_mut(&mut doc, &candidate.route)?;
        match crate::model_schema::write_provider(table, &schema, &[]) {
            Ok(()) => result.imported += 1,
            Err(e) => {
                crate::logging::warn("导入: 写入供应商失败", &e);
                result.failed += 1;
            }
        }
    }

    if result.imported > 0 {
        crate::model_manifest::write_config_doc(&doc)?;
    }
    Ok(result)
}

#[tauri::command]
pub fn model_import_scan() -> Vec<ImportGroup> {
    scan()
}

#[tauri::command]
pub fn model_import_run(keys: Vec<String>) -> Result<ImportRunResult, String> {
    run(&keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(id: &str, name: &str, base: Option<&str>, cred: Credential) -> Draft {
        Draft {
            id: id.to_string(),
            name: Some(name.to_string()),
            base_url: base.map(str::to_string),
            wire_api: None,
            credential: cred,
            models: Vec::new(),
        }
    }

    #[test]
    fn sanitize_route_normalizes_to_codex_charset() {
        assert_eq!(sanitize_route("DeepSeek"), "deepseek");
        assert_eq!(sanitize_route("my provider"), "my-provider");
        assert_eq!(sanitize_route("a...b"), "a-b");
        assert_eq!(sanitize_route("--x--"), "x");
        assert_eq!(sanitize_route("供应商"), "");
        assert_eq!(sanitize_route("a_b-c"), "a_b-c");
    }

    #[test]
    fn scan_codex_maps_credential_kinds() {
        let doc = concat!(
            "[model_providers.env-keyed]\n",
            "name = \"Env Keyed\"\n",
            "base_url = \"https://a.dev/v1\"\n",
            "env_key = \"A_KEY\"\n",
            "wire_api = \"responses\"\n",
            "[model_providers.literal]\n",
            "name = \"Literal\"\n",
            "base_url = \"https://b.dev/v1\"\n",
            "experimental_bearer_token = \"sk-secret\"\n",
            "[model_providers.none]\n",
            "name = \"None\"\n",
            "base_url = \"https://c.dev/v1\"\n",
        )
        .parse::<toml_edit::DocumentMut>()
        .unwrap();
        let drafts = scan_codex(&doc);
        assert_eq!(drafts.len(), 3);
        let by = |id: &str| drafts.iter().find(|d| d.id == id).unwrap();
        assert_eq!(by("env-keyed").credential, Credential::Env("A_KEY".into()));
        assert_eq!(by("env-keyed").wire_api.as_deref(), Some("responses"));
        assert_eq!(by("literal").credential, Credential::Literal);
        assert_eq!(by("none").credential, Credential::None);
        // 明文密钥值绝不进入草稿
        assert!(by("literal").base_url.is_some());
        let rendered = format!("{:?}", drafts);
        assert!(!rendered.contains("sk-secret"), "明文不得进入扫描结果");
    }

    #[test]
    fn scan_claude_code_requires_custom_base_url() {
        let tmp = std::env::temp_dir().join(format!("cpm-import-{}", std::process::id()));
        let dir = tmp.join(".claude");
        std::fs::create_dir_all(&dir).unwrap();

        // 没有自定义 base url → 不产候选
        std::fs::write(dir.join("settings.json"), r#"{"env":{"ANTHROPIC_API_KEY":"x"}}"#).unwrap();
        assert!(scan_claude_code(&tmp).is_empty());

        // 有 base url → 一个候选，且凭据判为明文（值不带走）
        std::fs::write(
            dir.join("settings.json"),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://relay.dev","ANTHROPIC_AUTH_TOKEN":"sk-secret"}}"#,
        )
        .unwrap();
        let drafts = scan_claude_code(&tmp);
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].base_url.as_deref(), Some("https://relay.dev"));
        assert_eq!(drafts[0].credential, Credential::Literal);
        assert!(!format!("{:?}", drafts).contains("sk-secret"));

        // 损坏 JSON → 静默跳过
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        assert!(scan_claude_code(&tmp).is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_opencode_maps_env_interpolation() {
        let tmp = std::env::temp_dir().join(format!("cpm-oc-{}", std::process::id()));
        let dir = tmp.join(".config").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("opencode.json"),
            r#"{
              "provider": {
                "relay": {
                  "name": "Relay",
                  "options": { "baseURL": "https://relay.dev/v1", "apiKey": "{env:RELAY_KEY}" },
                  "models": { "m1": {}, "m2": {} }
                },
                "plain": { "options": { "baseURL": "https://plain.dev/v1", "apiKey": "sk-live" } }
              }
            }"#,
        )
        .unwrap();
        let drafts = scan_opencode(&tmp);
        let relay = drafts.iter().find(|d| d.id == "relay").unwrap();
        assert_eq!(relay.credential, Credential::Env("RELAY_KEY".into()));
        assert_eq!(relay.models, vec!["m1".to_string(), "m2".to_string()]);
        let plain = drafts.iter().find(|d| d.id == "plain").unwrap();
        assert_eq!(plain.credential, Credential::Literal);
        assert!(!format!("{:?}", drafts).contains("sk-live"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scan_groups_are_fixed_order_and_complete() {
        let groups = scan();
        assert_eq!(
            groups.iter().map(|g| g.source.as_str()).collect::<Vec<_>>(),
            SOURCES.to_vec(),
            "来源顺序固定，缺失来源保留空组"
        );
    }

    #[test]
    fn sanitize_collision_keeps_first_entry_only() {
        // 直接验证去重规则（不影响真实磁盘扫描）
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let drafts = [
            draft("Deep Seek", "A", None, Credential::None),
            draft("deep-seek", "B", None, Credential::None),
            draft("other", "C", None, Credential::None),
        ];
        let kept: Vec<String> = drafts
            .iter()
            .filter(|d| {
                let r = sanitize_route(&d.id);
                !r.is_empty() && seen.insert(r)
            })
            .map(|d| d.name.clone().unwrap_or_default())
            .collect();
        assert_eq!(kept, vec!["A".to_string(), "C".to_string()]);
    }

    #[test]
    fn truncate_caps_long_names() {
        let long = "x".repeat(200);
        assert_eq!(truncate(&long, MAX_NAME_LENGTH).chars().count(), MAX_NAME_LENGTH);
        assert_eq!(truncate("short", MAX_NAME_LENGTH), "short");
        // 多字节字符按字符数截断，不切开
        let cjk = "模".repeat(200);
        assert_eq!(truncate(&cjk, 10), "模".repeat(10));
    }

    #[test]
    fn run_skips_existing_and_builtin_routes() {
        // 只断言判定逻辑：内置 id 与已存在路由都计入 skipped
        let doc = "[model_providers.taken]\nname = \"T\"\n"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        assert!(crate::model_schema::provider_defined(&doc, "taken"));
        assert!(crate::model_schema::provider_defined(
            &doc,
            crate::model_schema::BUILTIN_PROVIDER
        ));
        assert!(!crate::model_schema::provider_defined(&doc, "fresh"));
    }
}
