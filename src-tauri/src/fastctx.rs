//! FastCtx 集成：接入/摘除一律委托 fastctx CLI（ADR 0003），
//! 启动器不写 fastctx 拥有的 TOML 键；接入状态以 config.toml 的
//! [mcp_servers.fastctx] 为唯一事实来源，不存开关布尔值。

use serde::Serialize;
use std::process::Command;

use crate::config;
use crate::i18n::trf;

/// 安装检测 + 接入状态
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FastctxStatus {
    /// PATH 中 fastctx 可执行（--version 调用成功）
    pub installed: bool,
    /// fastctx --version 输出
    pub version: Option<String>,
    /// ~/.codex/config.toml 含 [mcp_servers.fastctx]
    pub integrated: bool,
    /// npm 最新版本号；仅当比已装版本新时返回（UI 右侧更新胶囊），否则 None
    pub latest_version: Option<String>,
}

/// 接入结果（apply 成功后的 status 自检）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    /// status 自检是否通过
    pub self_check_passed: bool,
    /// status 输出（自检失败时供 UI 展示摘要）
    pub self_check_output: String,
}

/// 构造一条 CLI 调用。
/// Windows 上 npm 全局包是 .cmd 批处理（npm 自身也是 npm.cmd），
/// CreateProcess 不能直接执行批处理，必须经 cmd /c（由 cmd 做 PATHEXT
/// 解析），且不弹控制台窗口
fn cli_command(program: &str, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = Command::new("cmd");
        cmd.arg("/c").arg(program).args(args);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

/// 跑一次 fastctx 子命令；失败统一成 stderr（空则 stdout）文本
fn run_fastctx(args: &[&str]) -> Result<String, String> {
    let output = cli_command("fastctx", args)
        .output()
        .map_err(|e| trf("Cannot execute fastctx: {error} (please run npm install --global fastctx first)", &[("error", e.to_string())]))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// 接入状态唯一事实来源：config.toml 是否含 [mcp_servers.fastctx]
fn integrated_in(content: &str) -> Result<bool, String> {
    let doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| trf("Failed to parse config.toml: {error}", &[("error", e.to_string())]))?;
    Ok(doc
        .get("mcp_servers")
        .and_then(|t| t.get("fastctx"))
        .is_some())
}

fn read_integrated() -> Result<bool, String> {
    let path = config::home_dir()?.join(".codex").join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(c) => integrated_in(&c),
        Err(_) => Ok(false), // 文件缺失 = 未接入
    }
}

/// 检测：安装状态（PATH 探测）+ 接入状态（读 config.toml），每次实时，不落盘。
/// async 使 npm view 网络查询跑在 Tokio runtime，不阻塞 UI 主线程（同 updater::check_update 约定）
#[tauri::command]
pub async fn fastctx_detect() -> Result<FastctxStatus, String> {
    let (installed, version) = match cli_command("fastctx", &["--version"]).output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, if v.is_empty() { None } else { Some(v) })
        }
        _ => (false, None),
    };
    // 有可执行版时向 npm 查最新版，仅比已装版新才带上（网络/查询失败静默降级为 None）
    let latest_version = match &version {
        Some(cur) => latest_version(cur).ok().flatten(),
        None => None,
    };
    Ok(FastctxStatus {
        installed,
        version,
        integrated: read_integrated()?,
        latest_version,
    })
}

/// 查 npm 最新 fastctx 版本；返回 Some 仅当比已装版本新。网络不可达/解析失败回 None
fn latest_version(current: &str) -> Result<Option<String>, String> {
    let output = cli_command("npm", &["view", "fastctx", "version"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Ok(None);
    }
    let latest = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if latest.is_empty() {
        return Ok(None);
    }
    Ok(is_newer(&latest, current).then_some(latest))
}

/// 语义版本号比较：cur < latest 才算有更新；解析失败按无更新处理
fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// 解析 "1.2.3" → (1,2,3)；容忍 npm view 可能带 `v` 前缀。解析失败返回 None
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = v.trim().trim_start_matches('v').split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let (a, b, c) = (parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?);
    Some((a, b, c))
}

/// 安装：npm install --global fastctx（设置页开关在未检测到安装时自动触发）
#[tauri::command]
pub fn fastctx_install() -> Result<(), String> {
    let output = cli_command("npm", &["install", "--global", "fastctx"])
        .output()
        .map_err(|e| trf("Cannot execute npm: {error} (please install Node.js first)", &[("error", e.to_string())]))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// 接入：fastctx apply --yes（默认 Standard 档）；成功后 status 自检，
/// 自检失败只回报不视为接入失败（ADR 0003：警告不回滚）
#[tauri::command]
pub fn fastctx_apply() -> Result<ApplyResult, String> {
    run_fastctx(&["apply", "--yes"])?;
    match run_fastctx(&["status"]) {
        Ok(out) => Ok(ApplyResult {
            self_check_passed: !out.contains("[FAIL]"),
            self_check_output: out,
        }),
        Err(e) => Ok(ApplyResult {
            self_check_passed: false,
            self_check_output: e,
        }),
    }
}

/// 摘除：fastctx unapply --yes（杀受管进程、移除配置、删 ~/.fastctx
/// 受管数据；npm 全局包保留，可重新接入）
#[tauri::command]
pub fn fastctx_unapply() -> Result<(), String> {
    run_fastctx(&["unapply", "--yes"])?;
    Ok(())
}

/// 打开 fastctx 控制台（系统终端里跑 fastctx TUI；调档/jobs/更新都在那里）
#[tauri::command]
pub fn fastctx_open_console() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            // start 只把带引号的首参当窗口标题，裸首参会被当成命令执行
            // （曾因此跑出 fastctx cmd /k fastctx）；空标题 "" 是安全惯例
            .args(["/c", "start", "", "cmd", "/k", "fastctx"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| trf("Cannot open console: {error}", &[("error", e.to_string())]))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("osascript")
            .args([
                "-e",
                r#"tell application "Terminal" to activate"#,
                "-e",
                r#"tell application "Terminal" to do script "fastctx""#,
            ])
            .spawn()
            .map_err(|e| trf("Cannot open console: {error}", &[("error", e.to_string())]))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // ponytail: Linux 桌面终端无统一入口，x-terminal-emulator 是 Debian 系
        // 约定，其它环境请自行在终端跑 fastctx；有真实需求再补探测链
        Command::new("x-terminal-emulator")
            .args(["-e", "fastctx"])
            .spawn()
            .map_err(|e| trf("Cannot open console (please run fastctx in a terminal yourself): {error}", &[("error", e.to_string())]))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{integrated_in, is_newer, parse_version};

    #[test]
    fn integrated_detection() {
        // 命中：fastctx 表存在（apply 后的形态）
        assert!(integrated_in("[mcp_servers.fastctx]\ncommand = \"/home/u/.fastctx/bin/fastctx\"\n").unwrap());
        // 未命中：其它 MCP server 不算
        assert!(!integrated_in("[mcp_servers.other]\ncommand = \"x\"\n").unwrap());
        // 未命中：空文件 / 无 mcp_servers
        assert!(!integrated_in("model = \"gpt-5\"\n").unwrap());
        // 解析失败要报错而不是谎报未接入
        assert!(integrated_in("[mcp_servers\n").is_err());
    }

    #[test]
    fn version_compare() {
        // 有更新
        assert!(is_newer("1.2.3", "1.2.2"));
        assert!(is_newer("1.3.0", "1.2.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        // 相同 / 已装更新 → 无更新
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.2", "1.2.3"));
        // 任意一侧解析失败 → 无更新
        assert!(!is_newer("v1.2.3", "abc"));
        assert!(!is_newer("", "1.2.3"));
    }

    #[test]
    fn version_parse() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version(" 1.2.3 "), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("a.b.c"), None);
        assert_eq!(parse_version("1.2.x"), None);
    }
}
