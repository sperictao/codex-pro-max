//! FastCtx 集成：接入/摘除一律委托 fastctx CLI（ADR 0003），
//! 启动器不写 fastctx 拥有的 TOML 键；接入状态以 config.toml 的
//! [mcp_servers.fastctx] 为唯一事实来源，不存开关布尔值。

use serde::Serialize;
use std::process::Command;

use crate::i18n::trf;
use crate::AppState;
use tauri::State;

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
    let output = cli_command("fastctx", args).output().map_err(|e| {
        trf(
            "Cannot execute fastctx: {error} (please run npm install --global fastctx first)",
            &[("error", e.to_string())],
        )
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// FastCtx 自己拥有的 TOML 路径；Guard 不得把这些键纳入托管。
/// 比较按 TOML segment 边界进行，避免误伤 `fastctx_extra` 等相似键。
pub(crate) fn is_fastctx_owned_toml_path(path: &str) -> bool {
    [
        "mcp_servers.fastctx",
        "features.code_mode",
        "tool_output_token_limit",
    ]
    .iter()
    .any(|root| {
        path == *root
            || path
                .strip_prefix(root)
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// 接入状态唯一事实来源：config.toml 是否含 [mcp_servers.fastctx]
fn integrated_in(content: &str) -> Result<bool, String> {
    let doc = content.parse::<toml_edit::DocumentMut>().map_err(|e| {
        trf(
            "Failed to parse config.toml: {error}",
            &[("error", e.to_string())],
        )
    })?;
    Ok(doc
        .get("mcp_servers")
        .and_then(|t| t.get("fastctx"))
        .is_some())
}

fn read_integrated(paths: &crate::codex_guard::AppPaths) -> Result<bool, String> {
    let path = paths.codex_file("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(c) => integrated_in(&c),
        Err(_) => Ok(false), // 文件缺失 = 未接入
    }
}

/// 检测：安装状态（PATH 探测）+ 接入状态（读 config.toml），每次实时，不落盘
#[tauri::command]
pub fn fastctx_detect(state: State<'_, AppState>) -> Result<FastctxStatus, String> {
    let (installed, version) = match cli_command("fastctx", &["--version"]).output() {
        Ok(o) if o.status.success() => {
            let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
            (true, if v.is_empty() { None } else { Some(v) })
        }
        _ => (false, None),
    };
    Ok(FastctxStatus {
        installed,
        version,
        integrated: read_integrated(&state.paths)?,
    })
}

/// 安装：npm install --global fastctx（设置页开关在未检测到安装时自动触发）
#[tauri::command]
pub fn fastctx_install() -> Result<(), String> {
    let output = cli_command("npm", &["install", "--global", "fastctx"])
        .output()
        .map_err(|e| {
            trf(
                "Cannot execute npm: {error} (please install Node.js first)",
                &[("error", e.to_string())],
            )
        })?;
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
pub fn fastctx_apply(state: State<'_, AppState>) -> Result<ApplyResult, String> {
    let _write = state.guard_coordinator.try_guard_write()?;
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
pub fn fastctx_unapply(state: State<'_, AppState>) -> Result<(), String> {
    let _write = state.guard_coordinator.try_guard_write()?;
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
            .map_err(|e| {
                trf(
                    "Cannot open console (please run fastctx in a terminal yourself): {error}",
                    &[("error", e.to_string())],
                )
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::integrated_in;

    #[test]
    fn integrated_detection() {
        // 命中：fastctx 表存在（apply 后的形态）
        assert!(integrated_in(
            "[mcp_servers.fastctx]\ncommand = \"/home/u/.fastctx/bin/fastctx\"\n"
        )
        .unwrap());
        // 未命中：其它 MCP server 不算
        assert!(!integrated_in("[mcp_servers.other]\ncommand = \"x\"\n").unwrap());
        // 未命中：空文件 / 无 mcp_servers
        assert!(!integrated_in("model = \"gpt-5\"\n").unwrap());
        // 解析失败要报错而不是谎报未接入
        assert!(integrated_in("[mcp_servers\n").is_err());
    }

    #[test]
    fn fastctx_owned_paths_use_segment_boundaries() {
        use super::is_fastctx_owned_toml_path;
        for path in [
            "mcp_servers.fastctx",
            "mcp_servers.fastctx.command",
            "features.code_mode",
            "features.code_mode.direct_only_tool_namespaces",
            "tool_output_token_limit",
        ] {
            assert!(
                is_fastctx_owned_toml_path(path),
                "{path} should be reserved"
            );
        }
        for path in [
            "mcp_servers.fastctx_extra",
            "features.code_mode_extra",
            "other.tool_output_token_limit",
        ] {
            assert!(
                !is_fastctx_owned_toml_path(path),
                "{path} should remain Guard-owned"
            );
        }
    }
}
