//! 模型配置域：`~/.codex/config.toml` 模型域的可视化管理。
//!
//! 事实来源只有一个——config.toml。本域读写顶层 `model` / `model_provider` /
//! `model_reasoning_effort` 三键与 `[model_providers.*]` 表；预设库（一组可
//! 一键应用的组合）存启动器配置，因为 config.toml 没有这个概念。
//!
//! 与看守域共享 config.toml 但键集不相交：`model_context_window`、
//! `model_auto_compact_token_limit` 等归看守域，本域不碰。
//!
//! schema 细节（托管键、透传、校验）在 [`crate::model_schema`]；
//! 目录、凭据、连通性、导入分别在各自的模块里。

use serde::Serialize;

use crate::config::{self, ModelPreset};
use crate::model_manifest::{read_config_doc, write_config_doc};
use crate::model_schema::{
    self, ActiveModel, ActiveModelView, ProviderSchema, BUILTIN_PROVIDER,
};

/// `model_reasoning_effort` 的合法档位。
///
/// 官方参考表列 minimal/low/medium/high；真实 Codex 配置里已经在用
/// `max`（gpt-6-astra），`none` 则是"显式关闭推理"。这里取并集：
/// 列表偏窄会让应用层拒绝用户手上真实存在的配置。
pub(crate) const EFFORTS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

fn log_warn(action: &str, msg: &str) {
    crate::logging::warn(&format!("模型配置: {action}"), msg);
}

// ============ 视图 ============

/// 模型页视图：三键 + 供应商 + 预设 + 目录 + 凭据，一次取齐。
/// 视图不含任何密钥明文；供应商的密钥状态由 `hasBearerToken` 表达。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelConfigView {
    pub active: ActiveModelView,
    pub providers: Vec<ProviderSchema>,
    pub presets: Vec<ModelPreset>,
    /// 已配置路由在 models.dev 目录里的条目（缓存缺失时 missing = true）
    pub catalog: crate::model_catalog::CatalogView,
    /// 环境变量引用 → 就绪状态
    pub credentials: Vec<CredentialEntry>,
    /// 可选的推理档（UI 下拉直接用，避免前端再抄一份档位表）
    pub efforts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CredentialEntry {
    pub name: String,
    pub info: crate::model_schema::CredentialInfo,
}

// ============ 校验 ============

pub(crate) fn validate_effort(effort: &str) -> Result<(), String> {
    if effort.is_empty() || EFFORTS.contains(&effort) {
        return Ok(());
    }
    let err = crate::i18n::trf(
        "Invalid reasoning effort: {value}",
        &[("value", effort.to_string())],
    );
    log_warn("校验推理力度", &err);
    Err(err)
}

/// 应用前校验：provider 非默认时必须在 `[model_providers]` 中已定义
fn validate_provider_exists(doc: &toml_edit::DocumentMut, provider: &str) -> Result<(), String> {
    if provider.is_empty()
        || provider == BUILTIN_PROVIDER
        || model_schema::provider_defined(doc, provider)
    {
        return Ok(());
    }
    let err = crate::i18n::trf(
        "Model provider does not exist: {id}",
        &[("id", provider.to_string())],
    );
    log_warn("应用模型", &err);
    Err(err)
}

// ============ 命令：当前模型 ============

/// 应用当前模型：三键统一「空 = 回落默认（删键）」，provider 为内置 openai
/// 时同样删键。预设应用也走这里，不设第二条写入路径。
#[tauri::command]
pub fn model_apply(model: String, provider: String, effort: String) -> Result<(), String> {
    let active = ActiveModel {
        model: model.trim().to_string(),
        provider: provider.trim().to_string(),
        effort: effort.trim().to_string(),
    };
    validate_effort(&active.effort)?;

    let mut doc = read_config_doc()?;
    validate_provider_exists(&doc, &active.provider)?;
    model_schema::write_active(&mut doc, &active)?;
    write_config_doc(&doc)
}

// ============ 命令：供应商 ============

/// 保存供应商（新建或覆盖）。
///
/// `bearer_token` 语义：`None` = 保持磁盘原值（UI 从不回传真值，所以"没填"
/// 不等于"清除"）；`Some("")` = 显式清除；`Some(v)` = 写入。
#[tauri::command]
pub fn model_provider_save(
    mut provider: ProviderSchema,
    bearer_token: Option<String>,
    previous_unknown_keys: Option<Vec<String>>,
) -> Result<(), String> {
    let route = provider.route.trim().to_string();
    model_schema::validate_route(&route)?;
    provider.route = route.clone();

    let name = provider.name.clone().unwrap_or_default().trim().to_string();
    if name.is_empty() {
        let err = crate::i18n::tr("Provider name cannot be empty");
        log_warn("保存供应商", &err);
        return Err(err);
    }
    provider.name = Some(name);

    let base_url = provider.base_url.clone().unwrap_or_default().trim().to_string();
    model_schema::validate_base_url(&base_url)?;
    provider.base_url = Some(base_url);

    let wire_api = provider.wire_api.clone().unwrap_or_default().trim().to_string();
    model_schema::validate_wire_api(&wire_api)?;
    provider.wire_api = if wire_api.is_empty() {
        None
    } else {
        Some(wire_api)
    };

    // env_key 与直填密钥互斥：同时给出时报错，而不是静默丢掉一个
    let env_key = provider.env_key.clone().unwrap_or_default().trim().to_string();
    let literal = bearer_token.as_deref().map(str::trim).unwrap_or("");
    if !env_key.is_empty() && !literal.is_empty() {
        let err = crate::i18n::tr(
            "Choose one auth method: environment variable name or API key",
        );
        log_warn("保存供应商", &err);
        return Err(err);
    }
    provider.env_key = if env_key.is_empty() { None } else { Some(env_key) };

    for (label, value) in [
        ("request_max_retries", provider.request_max_retries),
        ("stream_max_retries", provider.stream_max_retries),
        ("stream_idle_timeout_ms", provider.stream_idle_timeout_ms),
        ("startup_timeout_ms", provider.startup_timeout_ms),
    ] {
        if let Some(v) = value {
            model_schema::validate_non_negative(label, v)?;
        }
    }

    let mut doc = read_config_doc()?;
    let table = model_schema::provider_table_mut(&mut doc, &route)?;

    let previous = previous_unknown_keys.unwrap_or_default();
    model_schema::write_provider(table, &provider, &previous)?;
    model_schema::apply_bearer_token(table, bearer_token.as_deref());
    write_config_doc(&doc)
}

/// 删除供应商。若它是当前活跃 provider，同时删 `model_provider` 键
/// （回落内置 openai）。引用它的预设不回滚（应用时校验报「供应商不存在」）。
#[tauri::command]
pub fn model_provider_delete(id: String) -> Result<(), String> {
    let id = id.trim().to_string();
    let mut doc = read_config_doc()?;
    if id == BUILTIN_PROVIDER || !model_schema::provider_defined(&doc, &id) {
        let err = crate::i18n::trf("Provider not found: {id}", &[("id", id.clone())]);
        log_warn("删除供应商", &err);
        return Err(err);
    }
    model_schema::remove_provider(&mut doc, &id);
    let active = model_schema::read_active(&doc);
    if active.provider == id {
        doc.remove("model_provider");
    }
    write_config_doc(&doc)
}

// ============ 命令：预设库 ============

/// 保存预设：id 空 = 新建（生成 uuid），否则整体更新
#[tauri::command]
pub fn model_preset_save(mut preset: ModelPreset) -> Result<(), String> {
    preset.label = preset.label.trim().to_string();
    preset.model = preset.model.trim().to_string();
    preset.provider = preset.provider.trim().to_string();
    preset.effort = preset.effort.trim().to_string();
    if preset.label.is_empty() {
        let err = crate::i18n::tr("Preset name cannot be empty");
        log_warn("保存预设", &err);
        return Err(err);
    }
    if preset.model.is_empty() {
        let err = crate::i18n::tr("Model id cannot be empty");
        log_warn("保存预设", &err);
        return Err(err);
    }
    validate_effort(&preset.effort)?;

    let mut cfg = config::load_config()?;
    if preset.id.is_empty() {
        // 保存时校验供应商存在，避免攒下注定应用失败的预设
        let doc = read_config_doc()?;
        validate_provider_exists(&doc, &preset.provider)?;
        preset.id = uuid::Uuid::new_v4().to_string();
        cfg.model_presets.push(preset);
    } else {
        let slot = cfg
            .model_presets
            .iter_mut()
            .find(|p| p.id == preset.id)
            .ok_or_else(|| {
                let err = crate::i18n::trf("Preset not found: {id}", &[("id", preset.id.clone())]);
                log_warn("保存预设", &err);
                err
            })?;
        *slot = preset;
    }
    config::save_config(&cfg)
}

#[tauri::command]
pub fn model_preset_delete(id: String) -> Result<(), String> {
    let mut cfg = config::load_config()?;
    let before = cfg.model_presets.len();
    cfg.model_presets.retain(|p| p.id != id);
    if cfg.model_presets.len() == before {
        let err = crate::i18n::trf("Preset not found: {id}", &[("id", id.clone())]);
        log_warn("删除预设", &err);
        return Err(err);
    }
    config::save_config(&cfg)
}

// ============ 命令：视图 ============

/// 模型页视图。目录读缓存（不联网）；过期由 UI 提示刷新。
#[tauri::command]
pub fn model_config_view() -> Result<ModelConfigView, String> {
    let doc = read_config_doc()?;
    let active = model_schema::read_active(&doc);
    let providers = model_schema::read_providers(&doc);

    let catalog = match crate::model_catalog::load_cache() {
        Some(file) => {
            let routes: Vec<String> = providers.iter().map(|p| p.route.clone()).collect();
            crate::model_catalog::view_for_routes(&file, &routes)
        }
        None => crate::model_catalog::missing_view(),
    };

    let credentials: Vec<CredentialEntry> =
        crate::model_credentials::describe_many(&crate::model_credentials::collect_env_keys(&providers))
            .into_iter()
            .map(|(name, info)| CredentialEntry { name, info })
            .collect();

    let cfg = config::load_config()?;
    Ok(ModelConfigView {
        active: ActiveModelView {
            model: active.model,
            provider: active.provider,
            effort: active.effort,
        },
        providers,
        presets: cfg.model_presets,
        catalog,
        credentials,
        efforts: EFFORTS.iter().map(|s| (*s).to_string()).collect(),
    })
}

// ============ 命令：目录 ============

/// 刷新 models.dev 目录快照（联网），返回筛选后的视图。
/// async：27MB 下载跑在 tokio 运行时上，不占同步 IPC 通道。
#[tauri::command]
pub async fn model_catalog_refresh() -> Result<crate::model_catalog::CatalogView, String> {
    let file = crate::model_catalog::refresh().await?;
    let doc = read_config_doc()?;
    let routes: Vec<String> = model_schema::read_providers(&doc)
        .iter()
        .map(|p| p.route.clone())
        .collect();
    Ok(crate::model_catalog::view_for_routes(&file, &routes))
}

// ============ 命令：凭据 ============

/// 查询一组环境变量引用的就绪状态
#[tauri::command]
pub fn model_credential_describe(names: Vec<String>) -> Vec<CredentialEntry> {
    crate::model_credentials::describe_many(&names)
        .into_iter()
        .map(|(name, info)| CredentialEntry { name, info })
        .collect()
}

/// 写入/清除 `~/.codex/.env` 中的一个变量（值为 None = 删除）
#[tauri::command]
pub fn model_credential_set(name: String, value: Option<String>) -> Result<(), String> {
    crate::model_credentials::set_user_env(&name, value.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_schema::{read_active, read_providers};

    fn doc(src: &str) -> toml_edit::DocumentMut {
        src.parse().unwrap()
    }

    #[test]
    fn validate_effort_accepts_real_world_values() {
        // 官方参考表 + 真实配置里出现的档位都必须放行
        for e in EFFORTS {
            assert!(validate_effort(e).is_ok(), "{e} 应合法");
        }
        assert!(validate_effort("").is_ok(), "空 = 删键");
        assert!(validate_effort("ultra").is_err());
        assert!(validate_effort("HIGH").is_err(), "档位大小写敏感");
    }

    #[test]
    fn validate_provider_exists_allows_builtin_and_empty() {
        let d = doc("[model_providers.k]\nname = \"K\"\n");
        assert!(validate_provider_exists(&d, "").is_ok());
        assert!(validate_provider_exists(&d, "openai").is_ok());
        assert!(validate_provider_exists(&d, "k").is_ok());
        assert!(validate_provider_exists(&d, "ghost").is_err());
    }

    #[test]
    fn apply_then_delete_active_provider_falls_back_to_builtin() {
        // 模拟 model_apply + model_provider_delete 的键段（不落盘）
        let mut d = doc("[model_providers.ds]\nname = \"DS\"\n");
        model_schema::write_active(
            &mut d,
            &ActiveModel {
                model: "deepseek-chat".into(),
                provider: "ds".into(),
                effort: "high".into(),
            },
        )
        .unwrap();
        assert_eq!(read_active(&d).provider, "ds");
        assert_eq!(read_providers(&d).len(), 1);

        model_schema::remove_provider(&mut d, "ds");
        if read_active(&d).provider == "ds" {
            d.remove("model_provider");
        }
        let active = read_active(&d);
        assert_eq!(active.provider, "", "活跃供应商被删 → 回落内置");
        assert_eq!(active.model, "deepseek-chat", "模型键不被连带清掉");
        assert!(read_providers(&d).is_empty());
        assert!(!d.to_string().contains("[model_providers"), "{}", d.to_string());
    }

    #[test]
    fn delete_non_active_provider_keeps_active_key() {
        let mut d = doc(concat!(
            "model_provider = \"keep\"\n",
            "[model_providers.keep]\nname = \"K\"\n",
            "[model_providers.drop]\nname = \"D\"\n",
        ));
        model_schema::remove_provider(&mut d, "drop");
        let active = read_active(&d);
        assert_eq!(active.provider, "keep", "删的非活跃项，活跃键不动");
        assert_eq!(read_providers(&d).len(), 1);
    }

    #[test]
    fn view_pieces_compose_over_real_document() {
        let d = doc(concat!(
            "model = \"gpt-6-astra\"\n",
            "model_provider = \"custom\"\n",
            "model_reasoning_effort = \"max\"\n",
            "\n[model_providers.custom]\n",
            "name = \"OpenAI\"\n",
            "requires_openai_auth = true\n",
            "supports_websockets = true\n",
            "wire_api = \"responses\"\n",
        ));
        let active = read_active(&d);
        assert_eq!(active.model, "gpt-6-astra");
        assert_eq!(active.effort, "max");
        let providers = read_providers(&d);
        assert_eq!(providers.len(), 1);
        // 真实配置里的两个未文档化键必须完整投影并保留
        assert_eq!(providers[0].requires_openai_auth, Some(true));
        assert_eq!(providers[0].supports_websockets, Some(true));
        assert!(crate::model_schema::provider_defined(&d, "custom"));
        assert_eq!(
            crate::model_credentials::collect_env_keys(&providers),
            Vec::<String>::new(),
            "没有 env_key 时凭据列表为空"
        );
    }

    #[test]
    fn effort_list_covers_documented_and_observed_values() {
        assert!(EFFORTS.contains(&"minimal"));
        assert!(EFFORTS.contains(&"high"));
        assert!(EFFORTS.contains(&"max"));
        assert!(EFFORTS.contains(&"none"));
        assert_eq!(EFFORTS.len(), 7, "档位表只有一处事实来源");
    }
}
