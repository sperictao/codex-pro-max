use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 启动器配置，持久化到 ~/.dashi-launcher/config.json
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
    #[serde(default = "default_true")]
    pub separate_window_mode: bool,
}

fn default_codex_path() -> String {
    #[cfg(target_os = "macos")]
    {
        "/Applications/ChatGPT.app".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        // Windows 上 Codex 桌面应用常见安装路径
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
        format!("{}\\Programs\\ChatGPT\\ChatGPT.exe", local_app_data)
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
            separate_window_mode: true,
        }
    }
}

/// 跨平台获取用户主目录
fn home_dir() -> Result<PathBuf, String> {
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
    Ok(home.join(".dashi-launcher").join("config.json"))
}

/// 加载配置文件，不存在则返回默认值
pub fn load_config() -> Result<LauncherConfig, String> {
    let path = config_file_path()?;
    if !path.exists() {
        return Ok(LauncherConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取配置文件失败: {}", e))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("解析配置文件失败: {}", e))
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
