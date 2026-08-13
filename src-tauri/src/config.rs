use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 启动器配置，持久化到 ~/.dashi-taskboard-launcher/config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherConfig {
    /// dashi-taskboard 仓库的本地路径
    #[serde(default)]
    pub taskboard_path: String,

    /// Node.js 可执行文件路径（留空则使用 PATH 中的 node）
    #[serde(default)]
    pub node_path: String,

    /// Codex 桌面应用路径
    #[serde(default = "default_codex_path")]
    pub codex_app_path: String,

    /// Taskboard HTTP 服务端口
    #[serde(default = "default_taskboard_port")]
    pub taskboard_port: u16,

    /// Taskboard HTTP 绑定地址
    #[serde(default = "default_taskboard_host")]
    pub taskboard_host: String,

    /// CDP 远程调试端口
    #[serde(default = "default_cdp_port")]
    pub cdp_port: u16,

    /// Taskboard instance token（taskboard v0.2.3 token 模式的路由前缀 + 鉴权标识）。
    /// 本地生成、跨重启保持稳定，server 与 injector 共用；前端不展示、不参与 merge_settings。
    #[serde(default)]
    pub instance_token: String,

    /// Taskboard instance secret：/health challenge 的 HMAC-SHA256 proof 校验密钥。
    /// 与 instance_token 同生命周期，只在本机配置落盘。
    #[serde(default)]
    pub instance_secret: String,

    /// 是否在启动时自动打开浏览器
    #[serde(default = "default_true")]
    pub auto_open: bool,

    /// 是否使用独立窗口模式（true）或重启模式（false）
    #[serde(default)]
    pub separate_window_mode: bool,

    /// 关闭窗口时是否最小化到系统托盘（false 则退出应用）
    #[serde(default)]
    pub minimize_to_tray_on_close: bool,

    /// 界面语言："system"（跟随系统）/ "en" / "zh-CN"
    #[serde(default = "default_language")]
    pub language: String,

    /// Codex 配置看守状态（见 codex_guard.rs）
    #[serde(default)]
    pub codex_guard: crate::codex_guard::CodexGuardState,
}

fn default_codex_path() -> String {
    #[cfg(target_os = "macos")]
    {
        "/Applications/ChatGPT.app".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        // 不预填猜测路径，由 detect_codex_app 探测真实安装位置
        String::new()
    }
    #[cfg(target_os = "linux")]
    {
        "/usr/bin/chatgpt".to_string()
    }
}

fn default_taskboard_port() -> u16 {
    47823
}

fn default_taskboard_host() -> String {
    "127.0.0.1".to_string()
}

fn default_cdp_port() -> u16 {
    9231
}

fn default_true() -> bool {
    true
}

fn default_language() -> String {
    "system".to_string()
}

/// 生成新的 instance token/secret 对（taskboard v0.2.3 token 模式）。
/// token 为 uuid v4（满足 /^[a-z0-9-]{16,128}$/i）；secret 为两个 uuid 拼接去连字符
/// 的 64 位 hex（满足 /^[a-f0-9-]{32,128}$/i）。本机凭据，非高安全场景，uuid 熵足够
fn generate_credentials() -> (String, String) {
    let token = uuid::Uuid::new_v4().to_string();
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    (token, secret)
}

/// 取有效 instance token/secret（首启生成并落盘，之后稳定复用）。
/// 校验失败（外部篡改/损坏）则重新生成，保证双进程 env 永远一致
pub fn ensure_instance_credentials(config: &mut LauncherConfig) -> Result<(String, String), String> {
    let valid_token = config.instance_token.len() >= 16
        && config.instance_token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let valid_secret = config.instance_secret.len() >= 32
        && config.instance_secret.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    if valid_token && valid_secret {
        return Ok((config.instance_token.clone(), config.instance_secret.clone()));
    }
    let (token, secret) = generate_credentials();
    config.instance_token = token.clone();
    config.instance_secret = secret.clone();
    save_config(config)?;
    Ok((token, secret))
}

/// Taskboard base URL（含 token 路由前缀），run_taskctl / Codex env 注入共用。
/// taskctl 的 normalizeBaseUrl 会剥尾斜杠，故此处不带尾斜杠
pub fn taskboard_url(host: &str, port: u16, token: &str) -> String {
    format!("http://{host}:{port}/{token}")
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            taskboard_path: String::new(),
            node_path: String::new(),
            codex_app_path: default_codex_path(),
            taskboard_port: default_taskboard_port(),
            taskboard_host: default_taskboard_host(),
            cdp_port: default_cdp_port(),
            instance_token: String::new(),
            instance_secret: String::new(),
            auto_open: true,
            separate_window_mode: false,
            minimize_to_tray_on_close: false,
            language: default_language(),
            codex_guard: Default::default(),
        }
    }
}

/// 跨平台获取用户主目录
pub fn home_dir() -> Result<PathBuf, String> {
    // Unix 使用 HOME，Windows 使用 USERPROFILE
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .map_err(|_| crate::i18n::tr("Cannot get HOME environment variable"))
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| crate::i18n::tr("Cannot get USERPROFILE environment variable"))
    }
}

/// 获取配置文件路径
pub fn config_file_path() -> Result<PathBuf, String> {
    let home = home_dir()?;
    Ok(home.join(".dashi-taskboard-launcher").join("config.json"))
}

/// 跨进程发布 Taskboard endpoint 的运行时描述文件。
/// Store/MSIX 激活无法继承启动器 env，taskctl 会从该用户级固定路径发现带 token 的 URL。
pub fn taskboard_runtime_file_path() -> Result<PathBuf, String> {
    let home = home_dir()?;
    Ok(home
        .join(".dashi-taskboard-launcher")
        .join("launcher-runtime.json"))
}

/// 剥掉 Windows `\\?\` 扩展路径前缀。
/// Tauri resource_dir 内部 canonicalize 的副作用；CreateProcess 的工作目录参数
/// 不认这个前缀，Node 拿到也别扭，统一剥成普通路径
pub fn strip_unc(s: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{}", rest);
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// 加载配置文件，不存在则返回默认值
pub fn load_config() -> Result<LauncherConfig, String> {
    let path = config_file_path()?;
    if !path.exists() {
        return Ok(LauncherConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| crate::i18n::trf("Failed to read config file: {error}", &[("error", e.to_string())]))?;
    let mut config: LauncherConfig = serde_json::from_str(&content)
        .map_err(|e| crate::i18n::trf("Failed to parse config file: {error}", &[("error", e.to_string())]))?;
    // 兼容旧配置里已存的 \\?\ 前缀路径
    config.taskboard_path = strip_unc(&config.taskboard_path);
    Ok(config)
}

/// 用新设置字段更新现有配置，保留 codex_guard 等非设置页字段不变
/// 防止设置页保存时把内存中过时的看守状态写回
pub fn merge_settings(current: &mut LauncherConfig, settings: &LauncherConfig) {
    current.taskboard_path = settings.taskboard_path.clone();
    current.node_path = settings.node_path.clone();
    current.codex_app_path = settings.codex_app_path.clone();
    current.taskboard_port = settings.taskboard_port;
    current.taskboard_host = settings.taskboard_host.clone();
    current.cdp_port = settings.cdp_port;
    current.auto_open = settings.auto_open;
    current.separate_window_mode = settings.separate_window_mode;
    current.minimize_to_tray_on_close = settings.minimize_to_tray_on_close;
    current.language = settings.language.clone();
}

/// 保存配置文件
pub fn save_config(config: &LauncherConfig) -> Result<(), String> {
    let path = config_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::i18n::trf("Failed to create config directory: {error}", &[("error", e.to_string())]))?;
    }
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| crate::i18n::trf("Failed to serialize config: {error}", &[("error", e.to_string())]))?;
    std::fs::write(&path, content)
        .map_err(|e| crate::i18n::trf("Failed to write config file: {error}", &[("error", e.to_string())]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::CodexGuardState;
    use std::collections::HashMap;

    fn sample_guard_state() -> CodexGuardState {
        use crate::codex_guard::GuardParamState;
        let mut params = HashMap::new();
        params.insert(
            "features.image_generation".to_string(),
            GuardParamState {
                value: Some(serde_json::json!(false)),
                applied: true,
                locked: true,
                last_checked: Some(12345),
                last_restored: Some(12340),
            },
        );
        CodexGuardState {
            enabled: true,
            params,
            files: Vec::new(),
        }
    }

    #[test]
    fn merge_settings_preserves_codex_guard() {
        let mut current = LauncherConfig::default();
        current.codex_guard = sample_guard_state();
        current.taskboard_path = "/old/path".to_string();
        current.auto_open = true;

        let settings = LauncherConfig {
            taskboard_path: "/new/path".to_string(),
            auto_open: false,
            cdp_port: 9999,
            ..Default::default()
        };

        merge_settings(&mut current, &settings);

        // 设置类字段被更新
        assert_eq!(current.taskboard_path, "/new/path");
        assert_eq!(current.auto_open, false);
        assert_eq!(current.cdp_port, 9999);

        // codex_guard 完整保留，没有被默认值覆盖
        assert_eq!(current.codex_guard.enabled, true);
        let p = current.codex_guard.params.get("features.image_generation").unwrap();
        assert_eq!(p.applied, true);
        assert_eq!(p.locked, true);
        assert_eq!(p.last_checked, Some(12345));
        assert_eq!(p.value, Some(serde_json::json!(false)));
    }

    #[test]
    fn merge_settings_updates_all_setting_fields() {
        let mut current = LauncherConfig::default();
        current.codex_guard = sample_guard_state();

        let settings = LauncherConfig {
            taskboard_path: "tp".to_string(),
            node_path: "np".to_string(),
            codex_app_path: "cap".to_string(),
            taskboard_port: 1111,
            taskboard_host: "0.0.0.0".to_string(),
            cdp_port: 2222,
            auto_open: false,
            separate_window_mode: true,
            minimize_to_tray_on_close: true,
            language: "zh-CN".to_string(),
            ..Default::default()
        };

        merge_settings(&mut current, &settings);

        assert_eq!(current.taskboard_path, "tp");
        assert_eq!(current.node_path, "np");
        assert_eq!(current.codex_app_path, "cap");
        assert_eq!(current.taskboard_port, 1111);
        assert_eq!(current.taskboard_host, "0.0.0.0");
        assert_eq!(current.cdp_port, 2222);
        assert_eq!(current.auto_open, false);
        assert_eq!(current.separate_window_mode, true);
        assert_eq!(current.minimize_to_tray_on_close, true);
        assert_eq!(current.language, "zh-CN");

        // codex_guard 不变
        assert_eq!(current.codex_guard.enabled, true);
        assert_eq!(current.codex_guard.params.len(), 1);
    }

    #[test]
    fn default_guard_state_is_disabled_and_empty() {
        let cfg = LauncherConfig::default();
        assert_eq!(cfg.codex_guard.enabled, false);
        assert!(cfg.codex_guard.params.is_empty());
    }

    #[test]
    fn taskboard_url_builds_token_prefix() {
        // token 模式下 API 路由在 /<token>/ 前缀下，URL 构造不能带尾斜杠（taskctl normalizeBaseUrl 会剥）
        assert_eq!(taskboard_url("127.0.0.1", 47823, "tok"), "http://127.0.0.1:47823/tok");
        assert_eq!(taskboard_url("0.0.0.0", 8080, "abc"), "http://0.0.0.0:8080/abc");
    }
}
