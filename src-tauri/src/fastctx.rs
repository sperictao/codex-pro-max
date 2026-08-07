//! FastCtx 集成：接入/摘除一律委托 fastctx CLI（ADR 0003），
//! 启动器不写 fastctx 拥有的 TOML 键；接入状态以 config.toml 的
//! [mcp_servers.fastctx] 为唯一事实来源，不存开关布尔值。

use serde::Serialize;
use std::process::Command;

use crate::config;

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

/// 构造一条 fastctx CLI 调用。
/// Windows 上 npm 全局包是 fastctx.cmd 批处理，CreateProcess 不能直接执行
/// 批处理，必须经 cmd /c（由 cmd 做 PATHEXT 解析），且不弹控制台窗口
fn fastctx_command(args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = Command::new("cmd");
        cmd.arg("/c").arg("fastctx").args(args);
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("fastctx");
        cmd.args(args);
        cmd
    }
}

/// 跑一次 fastctx 子命令；失败统一成 stderr（空则 stdout）文本
fn run_fastctx(args: &[&str]) -> Result<String, String> {
    let output = fastctx_command(args)
        .output()
        .map_err(|e| format!("无法执行 fastctx: {}（请先 npm install --global fastctx）", e))?;
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
        .map_err(|e| format!("config.toml 解析失败: {}", e))?;
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

/// 检测：安装状态（PATH 探测）+ 接入状态（读 config.toml），每次实时，不落盘
#[tauri::command]
pub fn fastctx_detect() -> Result<FastctxStatus, String> {
    let (installed, version) = match fastctx_command(&["--version"]).output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, if v.is_empty() { None } else { Some(v) })
        }
        _ => (false, None),
    };
    Ok(FastctxStatus {
        installed,
        version,
        integrated: read_integrated()?,
    })
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
            // start 的第一个引号参数是窗口标题，后面才是要跑的命令
            .args(["/c", "start", "fastctx", "cmd", "/k", "fastctx"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("无法打开控制台: {}", e))?;
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
            .map_err(|e| format!("无法打开控制台: {}", e))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // ponytail: Linux 桌面终端无统一入口，x-terminal-emulator 是 Debian 系
        // 约定，其它环境请自行在终端跑 fastctx；有真实需求再补探测链
        Command::new("x-terminal-emulator")
            .args(["-e", "fastctx"])
            .spawn()
            .map_err(|e| format!("无法打开控制台（请自行在终端运行 fastctx）: {}", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::integrated_in;

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
}
