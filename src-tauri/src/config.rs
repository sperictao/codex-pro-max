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

    /// 是否在启动时自动打开浏览器
    #[serde(default = "default_true")]
    pub auto_open: bool,

    /// 是否使用独立窗口模式（true）或重启模式（false）
    #[serde(default)]
    pub separate_window_mode: bool,

    /// 关闭窗口时是否最小化到系统托盘（false 则退出应用）
    #[serde(default)]
    pub minimize_to_tray_on_close: bool,

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

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            taskboard_path: String::new(),
            node_path: String::new(),
            codex_app_path: default_codex_path(),
            taskboard_port: default_taskboard_port(),
            taskboard_host: default_taskboard_host(),
            cdp_port: default_cdp_port(),
            auto_open: true,
            separate_window_mode: false,
            minimize_to_tray_on_close: false,
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
            .map_err(|_| "无法获取 HOME 环境变量".to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .map_err(|_| "无法获取 USERPROFILE 环境变量".to_string())
    }
}

/// 获取配置文件路径
pub fn config_file_path() -> Result<PathBuf, String> {
    let home = home_dir()?;
    Ok(home.join(".dashi-taskboard-launcher").join("config.json"))
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
        .map_err(|e| format!("读取配置文件失败: {}", e))?;
    let mut config: LauncherConfig = serde_json::from_str(&content)
        .map_err(|e| format!("解析配置文件失败: {}", e))?;
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
}

/// 保存配置文件
pub fn save_config(config: &LauncherConfig) -> Result<(), String> {
    let path = config_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建配置目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    std::fs::write(&path, content)
        .map_err(|e| format!("写入配置文件失败: {}", e))
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
}
