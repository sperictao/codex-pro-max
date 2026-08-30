//! 模型配置：~/.codex/config.toml 模型域的可视化管理（参考 CCursor 的 Model Config）。
//! config.toml 是唯一事实来源：当前模型读写顶层 model / model_provider / model_reasoning_effort，
//! 供应商直接增删 [model_providers.<id>] 表；预设库（快速切换的组合）存启动器配置。
//! 与看守域共享 config.toml 但键集不相交；无锁定语义，不轮询。

use serde::Serialize;
use toml_edit::DocumentMut;

use crate::codex_fs::{
    codex_file, get_toml_path, remove_toml_path, set_toml_path, write_with_backup,
};
use crate::config;
use crate::i18n::{tr, trf};

/// 内置供应商 id：model_provider 键回落到它时直接删键（codex 默认即 openai）
pub(crate) const BUILTIN_PROVIDER: &str = "openai";

/// model_reasoning_effort 的合法取值（codex 官方档位；空串 = 不设置/删键）
const EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh"];

fn log_warn(action: &str, msg: &str) {
    crate::logging::warn(&format!("模型配置: {action}"), msg);
}

// ============ 视图类型 ============

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub env_key: String,
    pub bearer_token: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigView {
    /// 顶层 model 键；空 = codex 默认模型
    pub model: String,
    /// 顶层 model_provider 键；空或 openai = 内置默认
    pub provider: String,
    /// 顶层 model_reasoning_effort 键；空 = 未设置
    pub effort: String,
    pub providers: Vec<ProviderView>,
    pub presets: Vec<config::ModelPreset>,
}

// ============ config.toml 读写 ============

/// 读 config.toml 为 DocumentMut；文件不存在按空文档处理（首次写入即创建）
fn read_config_doc() -> Result<DocumentMut, String> {
    let path = codex_file("config.toml")?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    content.parse::<DocumentMut>().map_err(|e| {
        let err = trf("Failed to parse config.toml: {error}", &[("error", e.to_string())]);
        crate::logging::error("模型配置: 解析 config.toml", &err);
        err
    })
}

fn write_config_doc(doc: &DocumentMut) -> Result<(), String> {
    let path = codex_file("config.toml")?;
    write_with_backup("config.toml", &path, &doc.to_string())
}

fn get_str(doc: &DocumentMut, path: &str) -> String {
    get_toml_path(doc, path)
        .and_then(|i| i.as_str())
        .unwrap_or("")
        .to_string()
}

fn provider_table_path(id: &str) -> String {
    format!("model_providers.{id}")
}

/// provider 是否已在 [model_providers] 中定义（内置 openai 恒视为存在）
fn provider_defined(doc: &DocumentMut, id: &str) -> bool {
    id == BUILTIN_PROVIDER || get_toml_path(doc, &provider_table_path(id)).is_some()
}

// ============ 校验 ============

fn validate_provider_id(id: &str) -> Result<(), String> {
    if id == BUILTIN_PROVIDER {
        let err = tr("openai is the built-in provider id and cannot be recreated");
        log_warn("保存供应商", &err);
        return Err(err);
    }
    let ok = !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        let err = tr("Provider id may only contain letters, digits, '-' and '_'");
        log_warn("保存供应商", &err);
        return Err(err);
    }
    Ok(())
}

fn validate_effort(effort: &str) -> Result<(), String> {
    if effort.is_empty() || EFFORTS.contains(&effort) {
        return Ok(());
    }
    let err = trf("Invalid reasoning effort: {value}", &[("value", effort.to_string())]);
    log_warn("校验推理力度", &err);
    Err(err)
}

/// 供应商/预设共用的应用前校验：provider 非默认时必须在 model_providers 中已定义
fn validate_provider_exists(doc: &DocumentMut, provider: &str) -> Result<(), String> {
    if provider.is_empty() || provider == BUILTIN_PROVIDER || provider_defined(doc, provider) {
        return Ok(());
    }
    let err = trf("Model provider does not exist: {id}", &[("id", provider.to_string())]);
    log_warn("应用模型", &err);
    Err(err)
}

// ============ 命令：当前模型 ============

/// 应用当前模型：三键统一「空 = 回落默认（删键）」，provider 为内置 openai 时同样删键。
/// 预设应用也走这里，不设第二条写入路径。
#[tauri::command]
pub fn model_apply(model: String, provider: String, effort: String) -> Result<(), String> {
    let model = model.trim().to_string();
    let provider = provider.trim().to_string();
    let effort = effort.trim().to_string();
    validate_effort(&effort)?;

    let mut doc = read_config_doc()?;
    validate_provider_exists(&doc, &provider)?;

    if model.is_empty() {
        remove_toml_path(&mut doc, "model");
    } else {
        set_toml_path(&mut doc, "model", toml_edit::value(model.as_str()))?;
    }
    if provider.is_empty() || provider == BUILTIN_PROVIDER {
        remove_toml_path(&mut doc, "model_provider");
    } else {
        set_toml_path(&mut doc, "model_provider", toml_edit::value(provider.as_str()))?;
    }
    if effort.is_empty() {
        remove_toml_path(&mut doc, "model_reasoning_effort");
    } else {
        set_toml_path(&mut doc, "model_reasoning_effort", toml_edit::value(effort.as_str()))?;
    }
    write_config_doc(&doc)
}

// ============ 命令：供应商 ============

#[tauri::command]
pub fn model_provider_save(
    id: String,
    name: String,
    base_url: String,
    env_key: String,
    bearer_token: String,
) -> Result<(), String> {
    let id = id.trim().to_string();
    validate_provider_id(&id)?;
    let name = name.trim().to_string();
    if name.is_empty() {
        let err = tr("Provider name cannot be empty");
        log_warn("保存供应商", &err);
        return Err(err);
    }
    let base_url = base_url.trim().to_string();
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        let err = tr("Base URL must start with http:// or https://");
        log_warn("保存供应商", &err);
        return Err(err);
    }
    let env_key = env_key.trim().to_string();
    let bearer_token = bearer_token.trim().to_string();
    if !env_key.is_empty() && !bearer_token.is_empty() {
        let err = tr("Choose one auth method: environment variable name or API key");
        log_warn("保存供应商", &err);
        return Err(err);
    }

    let mut doc = read_config_doc()?;
    let prefix = provider_table_path(&id);
    set_toml_path(&mut doc, &format!("{prefix}.name"), toml_edit::value(name.as_str()))?;
    set_toml_path(&mut doc, &format!("{prefix}.base_url"), toml_edit::value(base_url.as_str()))?;
    // 认证二选一：写其一即清另一；都空 = 无鉴权端点（如本地服务），两键都不留
    if !env_key.is_empty() {
        set_toml_path(&mut doc, &format!("{prefix}.env_key"), toml_edit::value(env_key.as_str()))?;
        remove_toml_path(&mut doc, &format!("{prefix}.experimental_bearer_token"));
    } else if !bearer_token.is_empty() {
        set_toml_path(
            &mut doc,
            &format!("{prefix}.experimental_bearer_token"),
            toml_edit::value(bearer_token.as_str()),
        )?;
        remove_toml_path(&mut doc, &format!("{prefix}.env_key"));
    } else {
        remove_toml_path(&mut doc, &format!("{prefix}.env_key"));
        remove_toml_path(&mut doc, &format!("{prefix}.experimental_bearer_token"));
    }
    write_config_doc(&doc)
}

/// 删除供应商；若它是当前活跃 provider，同时删 model_provider 键（回落内置 openai）。
/// 引用它的预设不回滚（应用时校验报「供应商不存在」）。
#[tauri::command]
pub fn model_provider_delete(id: String) -> Result<(), String> {
    let id = id.trim().to_string();
    let mut doc = read_config_doc()?;
    if !provider_defined(&doc, &id) || id == BUILTIN_PROVIDER {
        let err = trf("Provider not found: {id}", &[("id", id.clone())]);
        log_warn("删除供应商", &err);
        return Err(err);
    }
    remove_toml_path(&mut doc, &provider_table_path(&id));
    // [model_providers] 空了就连表头一起删，不留空节
    if get_toml_path(&doc, "model_providers")
        .and_then(|i| i.as_table_like())
        .is_some_and(|t| t.is_empty())
    {
        remove_toml_path(&mut doc, "model_providers");
    }
    if get_str(&doc, "model_provider") == id {
        remove_toml_path(&mut doc, "model_provider");
    }
    write_config_doc(&doc)
}

// ============ 命令：预设库 ============

/// 保存预设：id 空 = 新建（生成 uuid），否则整体更新
#[tauri::command]
pub fn model_preset_save(mut preset: config::ModelPreset) -> Result<(), String> {
    preset.label = preset.label.trim().to_string();
    preset.model = preset.model.trim().to_string();
    preset.provider = preset.provider.trim().to_string();
    preset.effort = preset.effort.trim().to_string();
    if preset.label.is_empty() {
        let err = tr("Preset name cannot be empty");
        log_warn("保存预设", &err);
        return Err(err);
    }
    if preset.model.is_empty() {
        let err = tr("Model id cannot be empty");
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
                let err = trf("Preset not found: {id}", &[("id", preset.id.clone())]);
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
        let err = trf("Preset not found: {id}", &[("id", id.clone())]);
        log_warn("删除预设", &err);
        return Err(err);
    }
    config::save_config(&cfg)
}

// ============ 命令：视图 ============

#[tauri::command]
pub fn model_config_view() -> Result<ModelConfigView, String> {
    let doc = read_config_doc()?;
    let provider = get_str(&doc, "model_provider");

    let mut providers = Vec::new();
    if let Some(table) = get_toml_path(&doc, "model_providers").and_then(|i| i.as_table_like()) {
        for (id, item) in table.iter() {
            let Some(t) = item.as_table_like() else { continue };
            let s = |key: &str| t.get(key).and_then(|i| i.as_str()).unwrap_or("").to_string();
            providers.push(ProviderView {
                id: id.to_string(),
                name: s("name"),
                base_url: s("base_url"),
                env_key: s("env_key"),
                bearer_token: s("experimental_bearer_token"),
                active: id == provider,
            });
        }
    }
    providers.sort_by(|a, b| a.id.cmp(&b.id));

    let cfg = config::load_config()?;
    Ok(ModelConfigView {
        model: get_str(&doc, "model"),
        provider,
        effort: get_str(&doc, "model_reasoning_effort"),
        providers,
        presets: cfg.model_presets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_fs::{get_toml_path, remove_toml_path, set_toml_path};

    fn doc(src: &str) -> DocumentMut {
        src.parse::<DocumentMut>().unwrap()
    }

    // —— apply 的键写入语义 ——（不落盘，只构造文档后走同一段逻辑）

    #[test]
    fn apply_writes_keys_and_openai_removes_provider_key() {
        let mut d = doc("model = \"old\"\nmodel_provider = \"deepseek\"\n");
        // 模拟 model_apply 的键段：openai → 删键
        remove_toml_path(&mut d, "model_provider");
        set_toml_path(&mut d, "model", toml_edit::value("gpt-5-codex")).unwrap();
        assert_eq!(get_str(&d, "model"), "gpt-5-codex");
        assert!(get_toml_path(&d, "model_provider").is_none());
        // 既有内容不破坏
        assert!(d.to_string().contains("model = \"gpt-5-codex\""));
    }

    #[test]
    fn provider_defined_checks_table() {
        let d = doc("[model_providers.deepseek]\nname = \"DeepSeek\"\n");
        assert!(provider_defined(&d, "deepseek"));
        assert!(provider_defined(&d, BUILTIN_PROVIDER));
        assert!(!provider_defined(&d, "nope"));
    }

    #[test]
    fn provider_table_path_is_dotted() {
        assert_eq!(provider_table_path("x"), "model_providers.x");
    }

    #[test]
    fn delete_active_provider_clears_model_provider_key() {
        let mut d = doc("model_provider = \"ds\"\n\n[model_providers.ds]\nname = \"DS\"\n");
        remove_toml_path(&mut d, "model_providers.ds");
        if get_toml_path(&d, "model_providers")
            .and_then(|i| i.as_table_like())
            .is_some_and(|t| t.is_empty())
        {
            remove_toml_path(&mut d, "model_providers");
        }
        if get_str(&d, "model_provider") == "ds" {
            remove_toml_path(&mut d, "model_provider");
        }
        let out = d.to_string();
        assert!(get_toml_path(&d, "model_provider").is_none());
        assert!(get_toml_path(&d, "model_providers.ds").is_none());
        assert!(get_toml_path(&d, "model_providers").is_none());
        assert!(!out.contains("[model_providers"));
    }

    #[test]
    fn delete_keeps_other_providers_and_provider_key() {
        let mut d = doc("model_provider = \"ds\"\n\n[model_providers.ds]\nname = \"DS\"\n\n[model_providers.k]\nname = \"K\"\n");
        remove_toml_path(&mut d, "model_providers.ds");
        assert!(get_str(&d, "model_provider") == "ds"); // 活跃键未指向被删项时不动
        assert!(provider_defined(&d, "k"));
    }

    // —— 校验 ——

    #[test]
    fn validate_provider_id_rules() {
        assert!(validate_provider_id("deepseek").is_ok());
        assert!(validate_provider_id("my_provider-2").is_ok());
        assert_eq!(
            validate_provider_id("openai").unwrap_err(),
            tr("openai is the built-in provider id and cannot be recreated")
        );
        assert!(validate_provider_id("").is_err());
        assert!(validate_provider_id("a.b").is_err());
        assert!(validate_provider_id("空格").is_err());
    }

    #[test]
    fn validate_effort_rules() {
        for e in ["minimal", "low", "medium", "high", "xhigh", ""] {
            assert!(validate_effort(e).is_ok());
        }
        assert!(validate_effort("ultra").is_err());
    }

    #[test]
    fn validate_provider_exists_fails_for_missing() {
        let d = doc("[model_providers.k]\nname = \"K\"\n");
        assert!(validate_provider_exists(&d, "").is_ok());
        assert!(validate_provider_exists(&d, "openai").is_ok());
        assert!(validate_provider_exists(&d, "k").is_ok());
        assert!(validate_provider_exists(&d, "ghost").is_err());
    }

    #[test]
    fn view_reads_providers_and_active_flag() {
        let d = doc("model = \"m1\"\nmodel_provider = \"ds\"\nmodel_reasoning_effort = \"low\"\n\n[model_providers.ds]\nname = \"DeepSeek\"\nbase_url = \"https://api.deepseek.com\"\nenv_key = \"DEEPSEEK_API_KEY\"\n\n[model_providers.loc]\nname = \"Local\"\nbase_url = \"http://127.0.0.1:8080\"\nexperimental_bearer_token = \"sk-x\"\n");
        let provider = get_str(&d, "model_provider");
        let mut providers = Vec::new();
        let table = get_toml_path(&d, "model_providers").and_then(|i| i.as_table_like()).unwrap();
        for (id, item) in table.iter() {
            let t = item.as_table_like().unwrap();
            let s = |k: &str| t.get(k).and_then(|i| i.as_str()).unwrap_or("").to_string();
            providers.push((id.to_string(), s("name"), s("env_key"), s("experimental_bearer_token"), id == provider));
        }
        assert_eq!(providers.len(), 2);
        let ds = providers.iter().find(|p| p.0 == "ds").unwrap();
        assert_eq!(ds.1, "DeepSeek");
        assert_eq!(ds.2, "DEEPSEEK_API_KEY");
        assert!(ds.4);
        let loc = providers.iter().find(|p| p.0 == "loc").unwrap();
        assert_eq!(loc.3, "sk-x");
        assert!(!loc.4);
        assert_eq!(get_str(&d, "model"), "m1");
        assert_eq!(get_str(&d, "model_reasoning_effort"), "low");
    }
}
