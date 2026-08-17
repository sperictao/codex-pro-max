//! DeepSeek Harness (dsh) 远程访问：Tailscale 一键配置。
//!
//! 远程访问架构：
//! ```text
//! 远程设备 (Tailscale 内网)
//!   ▼
//! https://<hostname>.ts.net   (tailscale serve, HTTPS 443)
//!   ▼
//! 127.0.0.1:3899              (dsh web + auth-capable Connection)
//! ```
//!
//! 安全边界：dsh 显式绑定 loopback；Tailscale Serve 注入调用者身份；
//! `dsh-client-connection-authz` 在 HTTP/WebSocket 入口消费
//! `dsh-auth-tailscale` 提供的 authorizer。Launcher 不改写 Host/Origin，也不伪造
//! loopback 身份。远程特权 API 默认保持拒绝。
//!
//! 跨平台：安装/检测走 CLI（npm / tailscale / node），
//! 插件从应用内置 tarball 安装，开机自启走 launchd(macOS) /
//! 启动文件夹 .vbs(Windows) / systemd --user(Linux)。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::config;
use crate::i18n::{current, tr, trf};
use crate::version::parse_version;

/// Launcher 锁定的 dsh/auth 插件兼容栈。
const SUPPORTED_DSH_VERSION: &str = "0.1.0-rc.6";
const SUPPORTED_DSH_PACKAGE: &str = "@deepseek-ai/dsh@0.1.0-rc.6";
const CONNECTION_PLUGIN_PACKAGE: &str = "@dsh-external/dsh-client-connection-authz";
const AUTH_PLUGIN_PACKAGE: &str = "@dsh-external/dsh-auth-tailscale";
const CONNECTION_PLUGIN_TARBALL: &str = "dsh-client-connection-authz-873c465f1403.tgz";
const AUTH_PLUGIN_TARBALL: &str = "dsh-auth-tailscale-ea7ca9fa3db4.tgz";
const TAILSCALE_LOGIN_ENV: &str = "DSH_TAILSCALE_ALLOWED_LOGINS";
const LOCAL_ONLY_LOGIN: &str = "local-only@localhost.invalid";

/// dsh web 端口。
const WEB_PORT: u16 = 3899;
/// 自启标签前缀（仅 macOS launchd 使用；Windows/Linux 用固定文件名）
#[cfg(target_os = "macos")]
const AUTOSTART_PREFIX: &str = "com.codexpromax.dsh";

// ============ 数据结构 ============

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DshStatus {
    pub node_available: bool,
    pub dsh_installed: bool,
    pub dsh_version: Option<String>,
    pub supported_version: String,
    pub dsh_compatible: bool,
    pub plugins_installed: bool,
    pub dsh_running: bool,
    pub tailscale_installed: bool,
    pub tailscale_online: bool,
    pub hostname: Option<String>,
    /// 本机回环地址（dsh web 正在运行且授权栈就绪时可用）
    pub local_url: Option<String>,
    pub url: Option<String>,
    pub magic_dns_enabled: bool,
    pub serve_configured: bool,
    pub autostart_enabled: bool,
    /// 检测过程中的错误信息（无则 None）
    pub error: Option<String>,
}

/// 时间轴节点事件（dsh-step），由 dsh_setup 逐步发出
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StepEvent {
    pub index: usize,
    pub id: String,
    /// running | done | failed | skipped
    pub state: String,
    pub detail: Option<String>,
    /// 问题描述（失败节点展示）
    pub problem: Option<String>,
    /// 解决方案（失败节点展示）
    pub solution: Option<String>,
}

// ============ 跨平台 CLI 辅助 ============

/// Windows 命令行列转义（CommandLineToArgvW 规则的最小实现）。
/// 入参都是绝对路径/简单参数：无空格无引号原样返回；有空格则加引号，
/// 内嵌引号以反斜杠转义。反斜杠本身是字面量，绝不多写（否则 `C:\Program
/// Files\...` 会被翻倍成 `C:\\Program Files\\...` 而无法执行）。
/// 仅在 Windows 构建被 cli_command 使用；macOS/Linux 上保留给单元测试
#[cfg_attr(not(windows), allow(dead_code))]
fn win_quote(s: &str) -> String {
    if !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    let escaped = s.replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

/// 拼一条 cmd /c 命令行。cmd 对 /c 后字符串的解析规则与 CreateProcess 不同：
/// 若字符串以引号开头，cmd 会剥掉首引号与行尾最后一个引号。因此必须把
/// 「已逐引号的程序+参数」整体再包一层引号（微软文档的标准做法）：
///   cmd /c ""C:\Program Files\Tailscale\tailscale.exe" status"
/// 若只让 std 自动引号程序路径，cmd 剥引号后会把带空格的路径拆碎。
#[cfg_attr(not(windows), allow(dead_code))]
fn win_cmd_line(program: &str, args: &[&str]) -> String {
    let mut line = win_quote(program);
    for a in args {
        line.push(' ');
        line.push_str(&win_quote(a));
    }
    format!("\"{}\"", line)
}

/// Windows 上 npm/全局包是 .cmd 批处理，CreateProcess 不能直接执行，
/// 必须经 cmd /c 由 cmd 做 PATHEXT 解析，且不弹控制台窗口（同 fastctx.rs）
fn cli_command(program: &str, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = Command::new("cmd");
        // raw_arg 跳过 std 的自动引号（std 的 CommandLineToArgvW 引号会与
        // cmd 的 /c 解析规则打架），整个命令行按 win_cmd_line 手工构造
        cmd.raw_arg("/c").raw_arg(&win_cmd_line(program, args));
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

/// GUI 应用（Finder 启动）PATH 很薄，探测常见 CLI 位置补进 PATH
fn probe_path() -> String {
    // home 仅 macOS/Linux 用于拼接 npm 全局路径；Windows 走 APPDATA/LOCALAPPDATA
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let home = config::home_dir().unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    #[cfg(target_os = "macos")]
    {
        parts.push("/opt/homebrew/bin".to_string());
        parts.push("/usr/local/bin".to_string());
        if let Some(h) = home.to_str() {
            parts.push(format!("{}/.npm-global/bin", h));
        }
    }
    #[cfg(windows)]
    {
        // npm 全局前缀默认在 %APPDATA%\npm（Roaming），不是 LOCALAPPDATA
        if let Ok(roaming) = std::env::var("APPDATA") {
            parts.push(format!("{}\\npm", roaming));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            parts.push(format!("{}\\npm", local));
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            parts.push(format!("{}\\nodejs", pf));
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(h) = home.to_str() {
            parts.push(format!("{}/.local/bin", h));
            parts.push(format!("{}/.npm-global/bin", h));
        }
        parts.push("/usr/local/bin".to_string());
    }
    parts.push("/usr/bin".to_string());
    parts.push("/bin".to_string());
    #[cfg(windows)]
    if let Ok(sys) = std::env::var("SystemRoot") {
        parts.push(sys);
    }
    if let Ok(cur) = std::env::var("PATH") {
        parts.push(cur);
    }
    parts.join(if cfg!(windows) { ";" } else { ":" })
}

/// 跑命令并捕获 (stdout, stderr, 成功)。命令经 probe PATH 执行
fn run_capture(program: &str, args: &[&str]) -> Result<(String, String, bool), String> {
    let output = cli_command(program, args)
        .env("PATH", probe_path())
        .output()
        .map_err(|e| trf("Cannot execute {program}: {error}", &[
            ("program", program.to_string()),
            ("error", e.to_string()),
        ]))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok((stdout, stderr, output.status.success()))
}

/// 在 probe PATH 中定位可执行文件（unix: command -v；windows: where）
fn which(program: &str) -> Option<String> {
    #[cfg(unix)]
    {
        let quoted = program.replace('\'', "'\\''");
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("command -v '{}'", quoted))
            .env("PATH", probe_path())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW：GUI 应用拉起 cmd 不能闪控制台窗口
        // （v0.2.1 修过同一问题，dsh 模块新增时漏带，回归）
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let out = Command::new("cmd")
            .args(["/c", "where", program])
            .creation_flags(CREATE_NO_WINDOW)
            .env("PATH", probe_path())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let first = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_string();
        if first.is_empty() { None } else { Some(first) }
    }
}

/// 端口是否已有进程监听（dsh 运行状态的权威判断，跨平台）
fn port_listening(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(400),
    )
    .is_ok()
}

/// 后台启动进程并立即返回（孤儿进程继续运行；日志重定向到文件）。
/// 返回子进程 PID，供启动等待期间探活（进程已死则提前失败，不干等超时）。
/// 说明：dsh web 是常驻服务，不能随启动器退出而被杀，
/// 这里不持有 Child 句柄，与 ProcessManager 的随窗停服务语义刻意不同
fn spawn_detached(program: &str, args: &[&str], envs: &[(&str, &str)], log: &Path) -> Result<u32, String> {
    let dir = log.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)
        .map_err(|e| {
            log::error!("[dsh 启动] 创建日志目录失败: {}", e);
            trf("Failed to create directory: {error}", &[("error", e.to_string())])
        })?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| {
            log::error!("[dsh 启动] 打开日志文件失败: {}", e);
            trf("Cannot open log file: {error}", &[("error", e.to_string())])
        })?;
    let mut cmd = cli_command(program, args);
    cmd.env("PATH", probe_path())
        .stdout(std::process::Stdio::from(file.try_clone().map_err(|e| {
            log::error!("[dsh 启动] 复制日志文件句柄失败: {}", e);
            e.to_string()
        })?))
        .stderr(std::process::Stdio::from(file));
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let child = cmd
        .spawn()
        .map_err(|e| {
            log::error!("[dsh 启动] 启动子进程失败: {}", e);
            trf("Cannot start process: {error}", &[("error", e.to_string())])
        })?;
    Ok(child.id())
}

/// 进程是否仍存活（unix: kill(pid,0)；windows: OpenProcess + 退出码 STILL_ACTIVE）
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
    }
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
            let mut code = 0u32;
            // STILL_ACTIVE = 259
            let alive = GetExitCodeProcess(h, &mut code).is_ok() && code == 259;
            let _ = CloseHandle(h);
            alive
        }
    }
}

/// 等待 dsh web 就绪：轮询端口绑定；子进程已退出则提前返回失败（配合日志诊断快速报错）。
/// 首启可能较慢（首次初始化 / 杀软扫描新装的 dsh 包），进程活着就继续等到超时
fn wait_web_start(pid: Option<u32>, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if port_listening(WEB_PORT) {
            return true;
        }
        if let Some(pid) = pid {
            if !process_alive(pid) {
                return false;
            }
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    port_listening(WEB_PORT)
}

/// 按命令行特征杀进程（unix: pkill；windows: powershell）
fn kill_by_pattern(pattern: &str) {
    #[cfg(unix)]
    {
        let _ = Command::new("pkill").arg("-f").arg(pattern).output();
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW：停止/重启 dsh 时不能闪 powershell 控制台窗口
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let script = format!(
            "Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{p}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            p = pattern.replace('\'', "''")
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
}

/// 按命令行特征找 dsh web 进程 PID（"profile web --port 3899"）。
/// Windows 上 dsh web 是 `node ...\dsh\dist\index.js --profile web --port 3899`
/// （npm 包布局），同一特征三平台都能命中；找不到返回 None
fn dsh_web_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        let out = Command::new("pgrep")
            .arg("-f")
            .arg("profile web --port 3899")
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout).lines().next()?.trim().parse().ok()
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let script = "Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like '*profile web --port 3899*' } | Select-Object -First 1 -ExpandProperty ProcessId";
        let out = Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
}

// ============ 组件定位 ============

fn dsh_dir() -> Result<PathBuf, String> {
    Ok(config::home_dir()?.join(".dsh"))
}

#[derive(Debug, Clone)]
struct DshPluginSpecs {
    connection: String,
    auth: String,
}

fn plugin_file_spec(path: &Path) -> String {
    let normalized = config::strip_unc(&path.to_string_lossy()).replace('\\', "/");
    format!("file:{}", normalized)
}

fn bundled_plugin_tarball(app: &tauri::AppHandle, filename: &str) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Ok(resources) = app.path().resource_dir() {
        candidates.push(resources.join("dsh-plugins").join(filename));
        candidates.push(resources.join(filename));
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".artifacts")
            .join("dsh-plugins")
            .join(filename),
    );
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            log::error!("[dsh 插件] 内置插件 tarball 缺失: {}", filename);
            trf(
                "Bundled dsh plugin is missing: {plugin}",
                &[("plugin", filename.to_string())],
            )
        })
}

fn bundled_plugin_specs(app: &tauri::AppHandle) -> Result<DshPluginSpecs, String> {
    Ok(DshPluginSpecs {
        connection: plugin_file_spec(&bundled_plugin_tarball(app, CONNECTION_PLUGIN_TARBALL)?),
        auth: plugin_file_spec(&bundled_plugin_tarball(app, AUTH_PLUGIN_TARBALL)?),
    })
}

fn web_profile_package_path() -> Result<PathBuf, String> {
    Ok(dsh_dir()?.join("profiles").join("web").join("package.json"))
}

fn plugin_profile_is_current(contents: &str, connection_spec: &str, auth_spec: &str) -> bool {
    let Ok(package) = serde_json::from_str::<serde_json::Value>(contents) else {
        return false;
    };
    let dependencies = package
        .get("dependencies")
        .and_then(serde_json::Value::as_object);
    let bundles = package
        .pointer("/dsh/profile/bundles")
        .and_then(serde_json::Value::as_array);
    let has_dependency = |name: &str, spec: &str| {
        dependencies
            .and_then(|deps| deps.get(name))
            .and_then(serde_json::Value::as_str)
            == Some(spec)
    };
    let has_bundle = |name: &str| {
        bundles.is_some_and(|items| items.iter().any(|item| item.as_str() == Some(name)))
    };
    has_dependency(CONNECTION_PLUGIN_PACKAGE, connection_spec)
        && has_dependency(AUTH_PLUGIN_PACKAGE, auth_spec)
        && has_bundle(CONNECTION_PLUGIN_PACKAGE)
        && has_bundle(AUTH_PLUGIN_PACKAGE)
}

fn auth_plugins_installed(specs: &DshPluginSpecs) -> bool {
    let Ok(path) = web_profile_package_path() else {
        return false;
    };
    fs::read_to_string(path)
        .map(|contents| plugin_profile_is_current(&contents, &specs.connection, &specs.auth))
        .unwrap_or(false)
}

fn install_auth_plugins(app: &tauri::AppHandle) -> Result<DshPluginSpecs, String> {
    let specs = bundled_plugin_specs(app)?;
    if auth_plugins_installed(&specs) {
        return Ok(specs);
    }
    let dsh = resolve_dsh_bin()?;
    let dsh = dsh.display().to_string();
    match run_capture(
        &dsh,
        &[
            "plugin",
            "--profile",
            "web",
            "add",
            &specs.connection,
            &specs.auth,
        ],
    ) {
        Ok((_, _, true)) if auth_plugins_installed(&specs) => Ok(specs),
        Ok((_, err, true)) => {
            let e = trf(
                "dsh plugin install completed but the web profile is incomplete: {error}",
                &[("error", err)],
            );
            log::error!("[dsh 插件] profile 不完整: {}", e);
            Err(e)
        }
        Ok((_, err, false)) => {
            let e = trf(
                "Failed to install dsh auth plugins: {error}",
                &[(
                    "error",
                    if err.is_empty() {
                        "dsh plugin add failed".to_string()
                    } else {
                        err
                    },
                )],
            );
            log::error!("[dsh 插件] 安装失败: {}", e);
            Err(e)
        }
        Err(error) => {
            log::error!("[dsh 插件] 执行 dsh plugin add 失败: {}", error);
            Err(error)
        }
    }
}

/// 定位 node 可执行（绝对路径，供自启脚本嵌入）
fn resolve_node_bin() -> Result<String, String> {
    which("node").ok_or_else(|| {
        let err = tr("Node.js is not available; please install Node.js 18+ and restart this app");
        log::error!("[dsh] 定位 node 失败: {}", err);
        err
    })
}

/// 定位 npm（probe PATH 内；失败返回裸 "npm" 让错误自然暴露）
fn npm_bin() -> String {
    which("npm").unwrap_or_else(|| "npm".to_string())
}

/// 从 dsh --version 原始输出中提取可解析的版本号：容忍 "dsh 0.1.0"、"v0.1.0-rc.6"、
/// 尾部构建信息等前缀/杂质，保证版本胶囊显示与 semver 比较（version::is_newer）
/// 使用同一份干净版本号。提取失败回退原串（比较侧解析失败会安全降级为无更新）
fn normalize_version(raw: &str) -> String {
    let t = raw.trim();
    for tok in t.split_whitespace() {
        let tok = tok.trim_start_matches(['v', 'V']);
        if parse_version(tok).is_some() {
            return tok.to_string();
        }
    }
    t.to_string()
}

/// dsh --version 输出（经 normalize_version 规范化）；未安装返回 None
fn dsh_version() -> Option<String> {
    let bin = which("dsh")?;
    let (out, _, ok) = run_capture(&bin, &["--version"]).ok()?;
    if !ok {
        return None;
    }
    let v = normalize_version(&out);
    if v.is_empty() { None } else { Some(v) }
}

fn dsh_version_is_compatible(version: Option<&str>) -> bool {
    version == Some(SUPPORTED_DSH_VERSION)
}

/// 安装 Launcher 验证过的 dsh 版本，并在 npm 成功后再次校验实际 CLI。
fn install_supported_dsh() -> Result<String, String> {
    resolve_node_bin()?;
    match run_capture(&npm_bin(), &["install", "-g", SUPPORTED_DSH_PACKAGE]) {
        Ok((_, _, true)) => {}
        Ok((_, err, false)) => {
            let error = if err.is_empty() {
                format!("npm install -g {SUPPORTED_DSH_PACKAGE} failed")
            } else {
                err
            };
            log::error!("[dsh 安装] npm install -g 失败: {}", error);
            return Err(trf("Install failed: {error}", &[("error", error)]));
        }
        Err(error) => {
            log::error!("[dsh 安装] 执行 npm install 失败: {}", error);
            return Err(error);
        }
    }
    let version = dsh_version().ok_or_else(|| {
        let err = tr("dsh installed but cannot be located in PATH");
        log::error!("[dsh 安装] 安装后无法在 PATH 定位 dsh");
        err
    })?;
    if !dsh_version_is_compatible(Some(&version)) {
        let err = trf(
            "Installed dsh version {actual}, but this Launcher requires {expected}",
            &[
                ("actual", version),
                ("expected", SUPPORTED_DSH_VERSION.to_string()),
            ],
        );
        log::error!("[dsh 安装] 版本不兼容: {}", err);
        return Err(err);
    }
    Ok(SUPPORTED_DSH_VERSION.to_string())
}

/// 定位 dsh 可执行：先 probe PATH，再经 `npm prefix -g` 推 npm 全局 bin
fn resolve_dsh_bin() -> Result<PathBuf, String> {
    if let Some(p) = which("dsh") {
        return Ok(PathBuf::from(p));
    }
    if let Ok((out, _, ok)) = run_capture(&npm_bin(), &["prefix", "-g"]) {
        if ok {
            let prefix = PathBuf::from(out.trim());
            #[cfg(windows)]
            let candidates = [
                prefix.join("dsh.cmd"),
                prefix.join("dsh.ps1"),
                prefix.join("dsh"),
            ];
            #[cfg(not(windows))]
            let candidates = [prefix.join("bin").join("dsh")];
            for c in candidates {
                if c.exists() {
                    return Ok(c);
                }
            }
        }
    }
    Err({
        let err = tr("Cannot locate the dsh CLI; install it with npm install -g @deepseek-ai/dsh");
        log::error!("[dsh] 定位 dsh CLI 失败: {}", err);
        err
    })
}

/// 定位 tailscale CLI（Windows 默认装在 Program Files，不在 PATH）
fn tailscale_path() -> Option<String> {
    if let Some(p) = which("tailscale") {
        return Some(p);
    }
    #[cfg(windows)]
    for c in [
        "C:\\Program Files\\Tailscale\\tailscale.exe",
        "C:\\Program Files (x86)\\Tailscale\\tailscale.exe",
    ] {
        if Path::new(c).exists() {
            return Some(c.to_string());
        }
    }
    None
}

/// 解析 MagicDNS 状态与后缀（tailscale dns status）
fn magic_dns_info(ts: &str) -> (bool, Option<String>) {
    let Ok((out, _, _)) = run_capture(ts, &["dns", "status"]) else {
        return (false, None);
    };
    let enabled = out
        .lines()
        .any(|l| l.to_lowercase().contains("magicdns: enabled"));
    let suffix = out.lines().find_map(|l| {
        l.find("suffix = ")
            .map(|i| l[i + "suffix = ".len()..].trim_end_matches(')').trim().to_string())
    });
    (enabled, suffix)
}

/// 解析 tailnet 主机名与 HTTPS URL：
/// 1) tailscale serve status 里的 https://<host>.ts.net
/// 2) tailscale status 首行设备名 + MagicDNS 后缀
fn resolve_host_and_url() -> (Option<String>, Option<String>) {
    let Some(ts) = tailscale_path() else {
        return (None, None);
    };
    if let Ok((out, _, ok)) = run_capture(&ts, &["serve", "status"]) {
        if ok {
            for line in out.lines() {
                let l = line.trim();
                if let Some(rest) = l.strip_prefix("https://") {
                    let host = rest.split([' ', '/']).next().unwrap_or("");
                    if !host.is_empty() {
                        return (
                            Some(host.split('.').next().unwrap_or(host).to_string()),
                            Some(format!("https://{}", host)),
                        );
                    }
                }
            }
        }
    }
    if let Ok((out, _, ok)) = run_capture(&ts, &["status"]) {
        if ok {
            if let Some(first) = out.lines().next() {
                let host = first.split_whitespace().nth(1).unwrap_or("").to_string();
                if !host.is_empty() {
                    let (_, suffix) = magic_dns_info(&ts);
                    if let Some(sfx) = suffix {
                        return (Some(host.clone()), Some(format!("https://{}.{}", host, sfx)));
                    }
                    return (Some(host), None);
                }
            }
        }
    }
    (None, None)
}

fn tailscale_login_from_status_json(raw: &str) -> Result<String, String> {
    let status: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
        let err = trf(
            "Cannot parse Tailscale status: {error}",
            &[("error", e.to_string())],
        );
        log::error!("[dsh tailscale] 解析 status 失败: {}", err);
        err
    })?;
    let user_id = match status.pointer("/Self/UserID") {
        Some(serde_json::Value::Number(value)) => value.to_string(),
        Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
            value.trim().to_string()
        }
        _ => {
            let err = tr("Tailscale status does not contain the current user ID");
            log::error!("[dsh tailscale] {}", err);
            return Err(err);
        }
    };
    let login = status
        .get("User")
        .and_then(serde_json::Value::as_object)
        .and_then(|users| users.get(&user_id))
        .and_then(|user| user.get("LoginName"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            let err = tr("Tailscale status does not contain the current login name");
            log::error!("[dsh tailscale] {}", err);
            err
        })?;
    if !login
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "@._+-".contains(character))
    {
        let err = tr("Tailscale login name contains unsupported characters");
        log::error!("[dsh tailscale] {}", err);
        return Err(err);
    }
    Ok(login.to_string())
}

fn resolve_tailscale_login(ts: &str) -> Result<String, String> {
    match run_capture(ts, &["status", "--json"]) {
        Ok((out, _, true)) => tailscale_login_from_status_json(&out),
        Ok((_, err, false)) => {
            let e = trf(
                "Cannot read the current Tailscale identity: {error}",
                &[(
                    "error",
                    if err.is_empty() {
                        "tailscale status --json failed".to_string()
                    } else {
                        err
                    },
                )],
            );
            log::error!("[dsh tailscale] 读取身份失败: {}", e);
            Err(e)
        }
        Err(error) => {
            log::error!("[dsh tailscale] 执行 tailscale status 失败: {}", error);
            Err(error)
        }
    }
}

/// 只认可根路由直接指向 dsh web 端口的 Serve 配置。
fn serve_status_targets_web(status: &str) -> bool {
    let loopback = format!("http://127.0.0.1:{WEB_PORT}");
    let localhost = format!("http://localhost:{WEB_PORT}");
    status.lines().any(|line| {
        line.contains("proxy")
            && line.split_whitespace().any(|token| {
                let target = token.trim_end_matches('/');
                target == loopback || target == localhost
            })
    })
}

/// 解析 serve 是否已直接指向 dsh web。
fn serve_configured(ts: &str) -> bool {
    match run_capture(ts, &["serve", "status"]) {
        Ok((out, _, ok)) => ok && serve_status_targets_web(&out),
        Err(_) => false,
    }
}

/// 解析 tailnet 完全限定主机名（--trusted-host 用）：
/// 设备名 + MagicDNS 后缀，如 etmacminim4.taildde4.ts.net。
/// 后缀未知时省略：硬猜 `.ts.net` 可能是错的（实际后缀常是
/// taildde4.ts.net 之类）。
fn resolve_fqdn() -> Option<String> {
    let (host, _) = resolve_host_and_url();
    let host = host?;
    if host.contains('.') {
        return Some(host);
    }
    let suffix = tailscale_path().and_then(|ts| magic_dns_info(&ts).1);
    suffix.map(|s| format!("{}.{}", host, s))
}

/// tailscale 是否在线（tailscale status 成功即在线）
fn tailscale_online(ts: &str) -> bool {
    matches!(run_capture(ts, &["status"]), Ok((_, _, true)))
}

/// 极简 HTTP GET（本地验证用；不引网络库）
fn http_get(port: u16, host_header: &str, path: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    let mut s = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_secs(3),
    )
    .ok()?;
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host_header
    );
    s.write_all(req.as_bytes()).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    String::from_utf8_lossy(&buf)
        .lines()
        .next()
        .map(|l| l.to_string())
}

/// 状态码首行是否成功（200/3xx 均视为可达；dsh 根路径可能 302 到登录页）
fn http_ok(line: Option<&str>) -> bool {
    match line {
        Some(l) => {
            let status = l.split_whitespace().nth(1).unwrap_or("");
            status.starts_with('2') || status.starts_with('3')
        }
        None => false,
    }
}

/// 构造 JSON-RPC POST 请求（本地验证用）。Host 为 loopback、不带 Origin，
/// 专门验证「本机仍可访问特权 API」这条不变式。
fn rpc_request(method: &str) -> String {
    let body = format!(
        r#"{{"type":"client-request","rpcId":"t1","method":"{}","payload":{{}}}}"#,
        method
    );
    format!(
        "POST /api/{} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        method,
        body.len(),
        body
    )
}

/// 极简 RPC POST（本地验证用）：POST JSON-RPC 到本地端口，响应含
/// `"ok":true` 即通过。
fn rpc_ok(port: u16, method: &str) -> bool {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    let mut s = match TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_secs(3),
    ) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if s.write_all(rpc_request(method).as_bytes()).is_err() {
        return false;
    }
    if s.set_read_timeout(Some(Duration::from_secs(5))).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if s.read_to_end(&mut buf).is_err() {
        return false;
    }
    String::from_utf8_lossy(&buf).contains("\"ok\":true")
}

// ============ 检测 ============

#[tauri::command]
pub async fn dsh_detect(app: tauri::AppHandle) -> Result<DshStatus, String> {
    let (hostname, url) = resolve_host_and_url();
    let ts = tailscale_path();
    let (magic, _) = match &ts {
        Some(p) => magic_dns_info(p),
        None => (false, None),
    };
    let version = dsh_version();
    let dsh_compatible = dsh_version_is_compatible(version.as_deref());
    let (plugins_installed, plugin_error) = match bundled_plugin_specs(&app) {
        Ok(specs) => (auth_plugins_installed(&specs), None),
        Err(error) => {
            log::warn!("[dsh 检测] 定位内置插件失败: {}", error);
            (false, Some(error))
        }
    };
    let dsh_running = port_listening(WEB_PORT);
    let serve_configured = ts.as_deref().map(serve_configured).unwrap_or(false);
    let stack_ready = dsh_running && dsh_compatible && plugins_installed;
    let local_url = stack_ready.then(|| format!("http://127.0.0.1:{WEB_PORT}"));
    let url = if stack_ready && serve_configured {
        url
    } else {
        None
    };
    Ok(DshStatus {
        node_available: which("node").is_some(),
        dsh_installed: version.is_some(),
        dsh_version: version,
        supported_version: SUPPORTED_DSH_VERSION.to_string(),
        dsh_compatible,
        plugins_installed,
        dsh_running,
        tailscale_installed: ts.is_some(),
        tailscale_online: ts.as_deref().map(tailscale_online).unwrap_or(false),
        hostname,
        local_url,
        url,
        magic_dns_enabled: magic,
        serve_configured,
        autostart_enabled: autostart_enabled(),
        error: plugin_error,
    })
}

// ============ 一键启动（时间轴事件流） ============

fn emit_step(
    app: &tauri::AppHandle,
    index: usize,
    id: &str,
    state: &str,
    detail: Option<String>,
    problem: Option<String>,
    solution: Option<String>,
) {
    let _ = app.emit(
        "dsh-step",
        StepEvent {
            index,
            id: id.to_string(),
            state: state.to_string(),
            detail,
            problem,
            solution,
        },
    );
}

struct StepCtx<'a> {
    app: &'a tauri::AppHandle,
    index: usize,
    id: &'static str,
}

impl StepCtx<'_> {
    fn running(&self, detail: &str) {
        emit_step(self.app, self.index, self.id, "running", Some(detail.to_string()), None, None);
    }
    fn done(&self, detail: &str) {
        emit_step(self.app, self.index, self.id, "done", Some(detail.to_string()), None, None);
    }
    /// 失败：发出 failed 节点 + 把后续步骤标记 skipped，再返回 Err（时间轴即展示面）
    fn fail(&self, problem: &str, solution: &str, remaining: &[(&'static str, usize)]) -> Result<(), String> {
        log::error!("[dsh 一键配置] 步骤 {} 失败: {}", self.id, problem);
        emit_step(self.app, self.index, self.id, "failed", None, Some(problem.to_string()), Some(solution.to_string()));
        for (id, idx) in remaining {
            emit_step(self.app, *idx, id, "skipped", None, None, None);
        }
        Err(problem.to_string())
    }
}

/// 后台启动 dsh web 进程：显式绑定 loopback，并把 Tailscale 登录名交给
/// 授权插件。不等待端口就绪，返回子进程 PID 供启动等待探活；
/// 失败返回 (problem, solution) 供时间轴与更新流程分别展示针对性排障提示。
/// dsh_setup 与 dsh_update 共用
fn spawn_dsh_web(login: &str, fqdn: Option<&str>) -> Result<u32, (String, String)> {
    let dsh_bin = match resolve_dsh_bin() {
        Ok(b) => b,
        Err(e) => {
            log::error!("[dsh 启动] 定位 dsh CLI 失败: {}", e);
            return Err((e, tr("Install dsh first, then retry")));
        }
    };
    let mut args: Vec<String> = vec![
        "--profile".into(),
        "web".into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        WEB_PORT.to_string(),
    ];
    if let Some(fqdn) = fqdn.filter(|value| !value.is_empty()) {
        args.push("--trusted-host".into());
        args.push(fqdn.to_string());
    }
    let log = dsh_dir()
        .map(|d| d.join("dsh-web.log"))
        .unwrap_or_else(|_| PathBuf::from("dsh-web.log"));
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    spawn_detached(
        &dsh_bin.display().to_string(),
        &arg_refs,
        &[(TAILSCALE_LOGIN_ENV, login)],
        &log,
    )
    .map_err(|e| {
        log::error!("[dsh 启动] 拉起 dsh web 进程失败: {}", e);
        (
            e,
            tr("Port 3899 may be occupied; stop the process using it and retry"),
        )
    })
}

/// 读取日志末尾若干非空行（失败诊断用）；文件缺失/为空返回 None
fn read_log_tail(path: &Path, max_lines: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(max_lines);
    let tail = lines[start..].join("\n");
    if tail.trim().is_empty() { None } else { Some(tail) }
}

/// 启动失败诊断：把 dsh-web.log 尾部的真实错误带进时间轴（进程崩溃时这里就是
/// 堆栈），并按常见崩溃原因给出针对性方案。只读日志，不修改任何状态
fn start_failure_diagnosis(log: &Path) -> (String, String) {
    let tail = read_log_tail(log, 40);
    let problem = match &tail {
        Some(t) => {
            // 问题区只取前 8 行，避免长堆栈淹没时间轴
            let short: Vec<&str> = t.lines().take(8).collect();
            trf("dsh web failed to start; log says:\n{log}", &[("log", short.join("\n"))])
        }
        None => tr("dsh web failed to start (no log output; port 3899 may be occupied)"),
    };
    let solution = match &tail {
        // Windows 首启最典型崩溃：healProfilesModuleFallback 建符号链接被拒
        Some(t) if t.contains("EPERM") || t.contains("symlink") => {
            tr("dsh could not create symlinks; on Windows enable Developer Mode (Settings → Privacy & security → For developers), then retry")
        }
        _ => tr("Check the log at ~/.dsh/dsh-web.log; port 3899 may be occupied or the dsh CLI may need a newer Node.js"),
    };
    (problem, solution)
}

/// serve 配置失败时的针对性方案：错误文本含 TLS 证书类提示（教程 3.3 强调
/// HTTPS Certificates 是与 MagicDNS 独立的开关）→ 指向 admin/dns；否则 →
/// serve 首次启用授权链接
fn serve_failure_solution(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("tls cert") || e.contains("does not support") || e.contains("certificate") {
        tr("MagicDNS or HTTPS Certificates may not be enabled; open https://login.tailscale.com/admin/dns and enable MagicDNS and HTTPS Certificates, then retry")
    } else {
        tr("Open the authorization link in the error output to enable Serve for this tailnet (https://login.tailscale.com/f/serve), then retry")
    }
}

#[tauri::command]
pub async fn dsh_setup(app: tauri::AppHandle) -> Result<(), String> {
    let steps: [&'static str; 8] = [
        "node",
        "install",
        "plugins",
        "tailscale",
        "magicdns",
        "start",
        "serve",
        "verify",
    ];
    let remaining_after = |cur: usize| -> Vec<(&'static str, usize)> {
        steps
            .iter()
            .enumerate()
            .filter(|(index, _)| *index > cur)
            .map(|(index, id)| (*id, index))
            .collect()
    };

    {
        let ctx = StepCtx {
            app: &app,
            index: 0,
            id: steps[0],
        };
        ctx.running(&tr("Checking Node.js & npm…"));
        let node = match resolve_node_bin() {
            Ok(node) => node,
            Err(error) => return ctx.fail(
                &error,
                &tr("Install Node.js 18+ from https://nodejs.org, then restart this app and retry"),
                &remaining_after(0),
            ),
        };
        let (npm_version, _, npm_ok) = run_capture(&npm_bin(), &["--version"]).unwrap_or_default();
        let node_version = cli_command(&node, &["--version"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        let mut detail = format!("Node {}", node_version.trim());
        if npm_ok {
            detail.push_str(&format!(" · npm {}", npm_version.trim()));
        }
        ctx.done(&detail);
    }

    {
        let ctx = StepCtx {
            app: &app,
            index: 1,
            id: steps[1],
        };
        let current = dsh_version();
        if dsh_version_is_compatible(current.as_deref()) {
            ctx.done(&trf(
                "Compatible dsh is installed: {version}",
                &[("version", SUPPORTED_DSH_VERSION.to_string())],
            ));
        } else {
            ctx.running(&trf(
                "Installing compatible dsh {version}…",
                &[("version", SUPPORTED_DSH_VERSION.to_string())],
            ));
            match install_supported_dsh() {
                Ok(version) => ctx.done(&trf("Installed {version}", &[("version", version)])),
                Err(error) => {
                    return ctx.fail(
                        &error,
                        &trf(
                            "Check your network and npm settings, then run npm install -g {package} and retry",
                            &[("package", SUPPORTED_DSH_PACKAGE.to_string())],
                        ),
                        &remaining_after(1),
                    )
                }
            }
        }
    }

    {
        let ctx = StepCtx {
            app: &app,
            index: 2,
            id: steps[2],
        };
        let already_installed = bundled_plugin_specs(&app)
            .map(|specs| auth_plugins_installed(&specs))
            .unwrap_or(false);
        if already_installed {
            ctx.done(&tr("Authorization plugins are installed"));
        } else {
            ctx.running(&tr("Installing bundled dsh authorization plugins…"));
            if let Err(error) = install_auth_plugins(&app) {
                return ctx.fail(
                    &error,
                    &tr("Reinstall this Launcher if its bundled dsh plugins are missing, then retry"),
                    &remaining_after(2),
                );
            }
            ctx.done(&tr("Authorization plugins installed"));
        }
    }

    let tailscale = {
        let ctx = StepCtx {
            app: &app,
            index: 3,
            id: steps[3],
        };
        ctx.running(&tr("Checking Tailscale identity…"));
        let ts = match tailscale_path() {
            Some(ts) => ts,
            None => {
                return ctx.fail(
                    &tr("Tailscale is not installed"),
                    &tr("Install Tailscale and sign in, then run tailscale up and retry"),
                    &remaining_after(3),
                )
            }
        };
        if !tailscale_online(&ts) {
            return ctx.fail(
                &tr("Tailscale is not connected"),
                &tr("Run tailscale up and sign in with the account allowed to access dsh"),
                &remaining_after(3),
            );
        }
        let login = match resolve_tailscale_login(&ts) {
            Ok(login) => login,
            Err(error) => {
                return ctx.fail(
                    &error,
                    &tr("Update Tailscale, sign in again, and verify tailscale status --json"),
                    &remaining_after(3),
                )
            }
        };
        ctx.done(&trf(
            "Online · authorized identity: {login}",
            &[("login", login.clone())],
        ));
        (ts, login)
    };

    {
        let ctx = StepCtx {
            app: &app,
            index: 4,
            id: steps[4],
        };
        ctx.running(&tr("Checking MagicDNS…"));
        let (enabled, _) = magic_dns_info(&tailscale.0);
        if !enabled {
            return ctx.fail(
                &tr("MagicDNS is not enabled"),
                &tr("Open https://login.tailscale.com/admin/dns and enable MagicDNS and HTTPS Certificates, then retry"),
                &remaining_after(4),
            );
        }
        ctx.done(&tr("MagicDNS enabled"));
    }

    {
        let ctx = StepCtx {
            app: &app,
            index: 5,
            id: steps[5],
        };
        let fqdn = resolve_fqdn();
        if port_listening(WEB_PORT) && dsh_web_pid().is_none() {
            return ctx.fail(
                &tr("Port 3899 is occupied by another process"),
                &tr("Stop the process listening on 127.0.0.1:3899, then retry"),
                &remaining_after(5),
            );
        }
        if port_listening(WEB_PORT) {
            ctx.running(&tr("Restarting dsh web with authorization plugins…"));
            if let Err(error) = restart_dsh_web(&tailscale.1, fqdn.as_deref()) {
                return ctx.fail(
                    &error,
                    &tr("Check the log at ~/.dsh/dsh-web.log"),
                    &remaining_after(5),
                );
            }
        } else {
            ctx.running(&tr("Starting dsh web on 127.0.0.1:3899…"));
            let pid = match spawn_dsh_web(&tailscale.1, fqdn.as_deref()) {
                Ok(pid) => pid,
                Err((problem, solution)) => {
                    return ctx.fail(&problem, &solution, &remaining_after(5))
                }
            };
            if !wait_web_start(Some(pid), Duration::from_secs(60)) {
                let log = dsh_dir()
                    .map(|dir| dir.join("dsh-web.log"))
                    .unwrap_or_else(|_| PathBuf::from("dsh-web.log"));
                let (problem, solution) = start_failure_diagnosis(&log);
                return ctx.fail(&problem, &solution, &remaining_after(5));
            }
        }
        ctx.done(&tr("dsh web is running on 127.0.0.1:3899"));
    }

    {
        let ctx = StepCtx {
            app: &app,
            index: 6,
            id: steps[6],
        };
        ctx.running(&tr("Configuring Tailscale Serve directly to dsh…"));
        let result = run_capture(
            &tailscale.0,
            &["serve", "--https=443", "--bg", &WEB_PORT.to_string()],
        );
        match result {
            Ok((_, _, true)) if serve_configured(&tailscale.0) => {
                let (_, url) = resolve_host_and_url();
                match url {
                    Some(url) => ctx.done(&trf("HTTPS serve ready: {url}", &[("url", url)])),
                    None => ctx.done(&tr("HTTPS serve ready")),
                }
            }
            Ok((_, err, _)) => {
                let error = if err.is_empty() {
                    "tailscale serve failed".to_string()
                } else {
                    err
                };
                return ctx.fail(
                    &trf(
                        "Serve is not enabled or failed: {error}",
                        &[("error", error.clone())],
                    ),
                    &serve_failure_solution(&error),
                    &remaining_after(6),
                );
            }
            Err(error) => {
                return ctx.fail(
                    &error,
                    &tr("Run tailscale up first to sign in, then retry"),
                    &remaining_after(6),
                )
            }
        }
    }

    {
        let ctx = StepCtx {
            app: &app,
            index: 7,
            id: steps[7],
        };
        let (_, url) = resolve_host_and_url();
        let url_text = url
            .clone()
            .unwrap_or_else(|| "https://<hostname>.ts.net".to_string());
        ctx.running(&trf(
            "Verifying remote access ({url})…",
            &[("url", url_text.clone())],
        ));
        let web_ok = http_ok(http_get(WEB_PORT, "127.0.0.1", "/").as_deref());
        let plugins_ok = bundled_plugin_specs(&app)
            .map(|specs| auth_plugins_installed(&specs))
            .unwrap_or(false);
        let serve_ok = serve_configured(&tailscale.0);
        let https_ok = url.as_deref().map(https_endpoint_ok).unwrap_or(false);
        let ws_ok = url.as_deref().map(ws_endpoint_ok).unwrap_or(false);
        let local_privileged_ok = rpc_ok(WEB_PORT, "settings.describe");

        if web_ok && plugins_ok && serve_ok && https_ok && ws_ok && local_privileged_ok {
            ctx.done(&trf("Remote access is ready: {url}", &[("url", url_text)]));
        } else {
            let mut checks = Vec::new();
            if !web_ok {
                checks.push(tr("dsh web is not responding on 127.0.0.1:3899"));
            }
            if !plugins_ok {
                checks.push(tr("The dsh authorization plugin profile is incomplete"));
            }
            if !serve_ok {
                checks.push(tr("Tailscale Serve is not targeting 127.0.0.1:3899"));
            }
            if !https_ok {
                checks.push(trf(
                    "HTTPS endpoint is not responding: {url}",
                    &[("url", url_text.clone())],
                ));
            }
            if !ws_ok {
                checks.push(trf(
                    "WebSocket handshake failed: {url}/api/events.host",
                    &[("url", url_text.clone())],
                ));
            }
            if !local_privileged_ok {
                checks.push(tr("Local privileged API access failed on 127.0.0.1:3899"));
            }
            let separator = if current() == "zh-CN" { "；" } else { "; " };
            return ctx.fail(
                &tr("Verification failed; some components are not ready"),
                &checks.join(separator),
                &remaining_after(7),
            );
        }
    }

    if autostart_enabled() {
        autostart_impl(true).map_err(|e| {
            log::error!("[dsh 一键配置] 使能自启失败: {}", e);
            e
        })?;
    }
    Ok(())
}
// ============ 一键启动 dsh（纯本地链路） ============

/// 一键启动 dsh web 并返回本地访问地址（不碰 Tailscale Serve）。
/// 纯本地路径不安装授权插件（那是远程访问才需要的）；用不可能命中的
/// 远程登录名启动，真实 loopback 请求由 Connection 内置的本机能力直接放行。
#[tauri::command]
pub async fn dsh_start_web() -> Result<String, String> {
    let mut stack_changed = false;
    if !dsh_version_is_compatible(dsh_version().as_deref()) {
        install_supported_dsh()?;
        stack_changed = true;
    }
    if port_listening(WEB_PORT) {
        if dsh_web_pid().is_none() {
            let err = tr("Port 3899 is occupied by another process");
            log::error!("[dsh 一键启动] {}", err);
            return Err(err);
        }
        if stack_changed {
            restart_dsh_web(LOCAL_ONLY_LOGIN, None)?;
        }
        return Ok(format!("http://127.0.0.1:{}", WEB_PORT));
    }
    let pid = spawn_dsh_web(LOCAL_ONLY_LOGIN, None).map_err(|(problem, _)| {
        log::error!("[dsh 一键启动] 启动 dsh web 失败: {}", problem);
        problem
    })?;
    if !wait_web_start(Some(pid), Duration::from_secs(60)) {
        let log = dsh_dir()
            .map(|d| d.join("dsh-web.log"))
            .unwrap_or_else(|_| PathBuf::from("dsh-web.log"));
        let problem = start_failure_diagnosis(&log).0;
        log::error!("[dsh 一键启动] 等待 dsh web 就绪超时: {}", problem);
        return Err(problem);
    }
    Ok(format!("http://127.0.0.1:{}", WEB_PORT))
}

// ============ 更新 ============

fn runtime_auth_context() -> (String, Option<String>) {
    let Some(ts) = tailscale_path() else {
        return (LOCAL_ONLY_LOGIN.to_string(), None);
    };
    match resolve_tailscale_login(&ts) {
        Ok(login) => (login, resolve_fqdn()),
        Err(_) => (LOCAL_ONLY_LOGIN.to_string(), None),
    }
}

/// 修复 Launcher 锁定的 dsh + 授权插件兼容栈；若 web 正在运行则重启。
#[tauri::command]
pub async fn dsh_update(app: tauri::AppHandle) -> Result<String, String> {
    let was_running = port_listening(WEB_PORT);
    let version = install_supported_dsh()
        .map_err(|error| {
            log::error!("[dsh 修复] 安装 dsh 失败: {}", error);
            trf("Repair failed: {error}", &[("error", error)])
        })?;
    install_auth_plugins(&app)
        .map_err(|error| {
            log::error!("[dsh 修复] 安装授权插件失败: {}", error);
            trf("Repair failed: {error}", &[("error", error)])
        })?;
    if was_running {
        let (login, fqdn) = runtime_auth_context();
        restart_dsh_web(&login, fqdn.as_deref())?;
    }
    Ok(version)
}

/// 重启 dsh web，确保新 profile 和授权环境生效。
fn restart_dsh_web(login: &str, fqdn: Option<&str>) -> Result<(), String> {
    stop_supervised_services();
    kill_by_pattern("profile web --port 3899");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while port_listening(WEB_PORT) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
    }
    if port_listening(WEB_PORT) {
        let err = tr("dsh web did not release port 3899");
        log::error!("[dsh 重启] {}", err);
        return Err(err);
    }
    let pid = spawn_dsh_web(login, fqdn).map_err(|(problem, _)| {
        log::error!("[dsh 重启] 启动 dsh web 失败: {}", problem);
        problem
    })?;
    if !wait_web_start(Some(pid), Duration::from_secs(60)) {
        let log = dsh_dir()
            .map(|d| d.join("dsh-web.log"))
            .unwrap_or_else(|_| PathBuf::from("dsh-web.log"));
        let problem = start_failure_diagnosis(&log).0;
        log::error!("[dsh 重启] 等待 dsh web 就绪超时: {}", problem);
        return Err(problem);
    }
    Ok(())
}

/// 真实 HTTPS 端点检查：curl -k 请求本机自己的 tailnet 域名。
/// Windows 10 1803+ 自带 curl.exe；macOS/Linux 标配 curl。
/// 返回是否拿到 2xx/3xx 响应
fn https_endpoint_ok(url: &str) -> bool {
    let null_dev = if cfg!(windows) { "NUL" } else { "/dev/null" };
    match run_capture(
        "curl",
        &["-sk", "--max-time", "10", "-o", null_dev, "-w", "%{http_code}", url],
    ) {
        Ok((out, _, ok)) => {
            let code = out.trim();
            ok && (code.starts_with('2') || code.starts_with('3'))
        }
        Err(_) => false,
    }
}

/// WebSocket 握手探测脚本（node 一段式，net/tls 裸 upgrade——不依赖 v22+ 内置
/// WebSocket，Node 18+ 均可用；实测 ws/wss 成功、无监听、非 101 三类路径）。
/// 教程第七步的纠错：curl 默认 HTTP/2 禁 Upgrade 头，测 WS 握手会拿到假 426——
/// 必须发真实 upgrade 握手。拿到 HTTP/1.1 101 即 exit 0，否则/超时 exit 1
const WS_PROBE_JS: &str = r"const net=require('net'),tls=require('tls');
const url=new URL(process.argv[1]);
const port=url.port?Number(url.port):(url.protocol==='wss:'?443:80);
const opts={host:url.hostname,port:port};
if(url.protocol==='wss:'){opts.rejectUnauthorized=false;if(!/^\d{1,3}(\.\d{1,3}){3}$/.test(url.hostname)){opts.servername=url.hostname;}}
const sock=url.protocol==='wss:'?tls.connect(opts):net.connect(port,url.hostname);
const key='dGhlIHNhbXBsZSBub25jZQ==';
let done=false,sent=false,buf='';
function finish(c){if(done)return;done=true;try{sock.destroy();}catch(e){}process.exit(c);}
function send(){if(sent)return;sent=true;sock.write('GET '+url.pathname+' HTTP/1.1\r\nHost: '+url.host+'\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: '+key+'\r\nSec-WebSocket-Version: 13\r\n\r\n');}
sock.on('connect',send);
sock.on('secureConnect',send);
sock.on('data',function(d){buf+=d.toString('utf8');if(buf.indexOf('HTTP/1.1 ')!==0){return;}finish(buf.indexOf('HTTP/1.1 101')===0?0:1);});
sock.on('error',function(){finish(1);});
sock.on('close',function(){finish(1);});
setTimeout(function(){finish(1);},6000).unref();";

/// 用 node 跑 WS 探测脚本。ws:// 走纯 TCP，wss:// 走 TLS。
/// node 不可用时跳过（视为通过）——setup 第 0 步已确认 node。
fn ws_probe_ok(node: &str, ws_url: &str) -> bool {
    matches!(run_capture(node, &["-e", WS_PROBE_JS, ws_url]), Ok((_, _, true)))
}

/// 真实 WebSocket 链路检查：经 Tailscale Serve 直接到 dsh，对
/// /api/events.host 做 WS upgrade 握手。
fn ws_endpoint_ok(url: &str) -> bool {
    let Some(node) = which("node") else { return true };
    let ws_url = format!("{}/api/events.host", url.replacen("https://", "wss://", 1));
    ws_probe_ok(&node, &ws_url)
}

// ============ 停止 ============

/// 停止自启监管下的 dsh 服务（launchd / systemd --user）；best-effort。
/// 只停当前会话、不动开机自启配置：launchd 用不带 -w 的 unload（plist 保留，
/// 下次登录仍自启）；systemd stop ≠ disable，干净停止不触发 on-failure 重启。
/// Windows 自启是启动文件夹 .vbs（仅登录时跑一次，无 KeepAlive），无需处理
fn stop_supervised_services() {
    #[cfg(target_os = "macos")]
    {
        if let Ok(home) = config::home_dir() {
            let agents = home.join("Library/LaunchAgents");
            for name in ["web", "proxy"] {
                let plist = agents.join(format!("{}.{}.plist", AUTOSTART_PREFIX, name));
                if plist.exists() {
                    let _ = Command::new("launchctl").arg("unload").arg(&plist).output();
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", "dsh-remote-web.service", "dsh-remote-proxy.service"])
            .output();
    }
}

#[tauri::command]
pub fn dsh_stop() -> Result<(), String> {
    // 先停自启监管再杀进程，避免 KeepAlive/Restart 立即拉活。
    stop_supervised_services();
    // 兜底：未经自启机制、由一键启动直接拉起的游离进程。
    // 注意：Windows 上 dsh web 的进程命令行是
    // `node ...\dsh\dist\index.js --profile web --port 3899`（npm 包布局），
    // 不以 "dsh --profile" 开头；"profile web --port 3899" 在 macOS 直启、
    // launchd 与 Windows shim 三条路径都能命中，且不以 `-` 开头（pkill 不会误当选项）
    kill_by_pattern("profile web --port 3899");
    // 迁移清理：旧 Launcher 可能留下运行中的 loopback 反代。
    kill_by_pattern("loopback-proxy.js");
    // 一并关闭 HTTPS Serve，避免远程 URL 指向已停止的 dsh。
    if let Some(ts) = tailscale_path() {
        let _ = run_capture(&ts, &["serve", "--https=443", "off"]);
    }
    Ok(())
}

// ============ 开机自启（launchd / 启动文件夹 / systemd --user） ============

/// sh 单引号转义（生成的启动脚本内嵌绝对路径）
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 端口占用守卫脚本（node 一段式）：端口已被监听 → exit 0（自启服务不重复启动）
fn port_guard_js(port: u16) -> String {
    format!(
        "const net=require('net');const s=net.connect({port},'127.0.0.1');s.on('connect',()=>process.exit(0));s.on('error',()=>process.exit(1));setTimeout(()=>process.exit(1),1000).unref();",
        port = port
    )
}

/// 生成 dsh web 启动脚本（自启用）：带端口守卫、loopback 绑定、
/// Tailscale 授权身份与 --trusted-host。
#[cfg_attr(windows, allow(dead_code))]
fn render_start_web(node: &str, dsh: &str, host: &str, login: &str) -> String {
    let trusted = if host.is_empty() {
        String::new()
    } else {
        format!(" --trusted-host {}", sh_quote(host))
    };
    // dsh 是 npm shim（#!/usr/bin/env node）：launchd/systemd 的裸 PATH 下
    // env 找不到 node，shim 以 127 秒退（实机踩坑：开机自启从未真正拉起
    // dsh web）。把 node 所在目录补进 PATH，shim 才能解析到解释器
    let node_dir = Path::new(node)
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    format!(
        "#!/bin/sh\n# generated by Codex Pro Max; do not edit\nif {node} -e {guard}; then exit 0; fi\nexport PATH={node_dir}:$PATH\nexport {login_env}={login}\nexec {dsh} --profile web --host 127.0.0.1 --port {port}{trusted}\n",
        node = sh_quote(node),
        guard = sh_quote(&port_guard_js(WEB_PORT)),
        node_dir = sh_quote(&node_dir),
        login_env = TAILSCALE_LOGIN_ENV,
        login = sh_quote(login),
        dsh = sh_quote(dsh),
        port = WEB_PORT,
        trusted = trusted,
    )
}

/// XML 转义（plist 内容，仅 macOS launchd 使用）
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn autostart_enabled() -> bool {
    #[cfg(target_os = "macos")]
    {
        config::home_dir()
            .map(|h| {
                h.join("Library/LaunchAgents")
                    .join(format!("{}.web.plist", AUTOSTART_PREFIX))
                    .exists()
            })
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        // 启动文件夹里的 .vbs 是否存在
        windows_startup_dir()
            .map(|d| d.join("dsh-remote-autostart.vbs").exists())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        // systemd unit 与 XDG autostart .desktop 任一存在即视为已开启
        config::home_dir()
            .map(|h| {
                h.join(".config/systemd/user")
                    .join("dsh-remote-web.service")
                    .exists()
                    || h.join(".config/autostart")
                        .join("dsh-remote-web.desktop")
                        .exists()
            })
            .unwrap_or(false)
    }
}

#[tauri::command]
pub fn dsh_set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        if !dsh_version_is_compatible(dsh_version().as_deref()) {
            install_supported_dsh()?;
        }
        install_auth_plugins(&app)?;
    }
    autostart_impl(enabled).map_err(|e| {
        log::error!("[dsh 自启] {} 失败: {}", if enabled { "开启" } else { "关闭" }, e);
        e
    })
}

#[cfg(target_os = "macos")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let home = config::home_dir()?;
    let agents_dir = home.join("Library/LaunchAgents");
    let dsh = dsh_dir()?;
    let web_plist = agents_dir.join(format!("{}.web.plist", AUTOSTART_PREFIX));
    let legacy_proxy_plist = agents_dir.join(format!("{}.proxy.plist", AUTOSTART_PREFIX));
    let web_script = dsh.join("start-web.sh");
    let legacy_proxy_script = dsh.join("start-proxy.sh");

    let unload = |label: &str, plist: &Path| {
        let _ = Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(plist)
            .output();
        let _ = Command::new("launchctl").arg("remove").arg(label).output();
    };
    unload(&format!("{}.proxy", AUTOSTART_PREFIX), &legacy_proxy_plist);
    let _ = fs::remove_file(&legacy_proxy_plist);
    let _ = fs::remove_file(&legacy_proxy_script);
    kill_by_pattern("loopback-proxy.js");

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let ts = tailscale_path().ok_or_else(|| tr("Tailscale is not installed"))?;
        let login = resolve_tailscale_login(&ts)?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        fs::create_dir_all(&agents_dir).map_err(|error| {
            log::error!("[dsh 自启(mac)] 创建 LaunchAgents 目录失败: {}", error);
            trf(
                "Failed to create directory: {error}",
                &[("error", error.to_string())],
            )
        })?;
        fs::write(
            &web_script,
            render_start_web(&node, &dsh_bin.display().to_string(), &fqdn, &login),
        )
        .map_err(|error| {
            log::error!("[dsh 自启(mac)] 写启动脚本失败: {}", error);
            trf(
                "Failed to write {path}: {error}",
                &[
                    ("path", web_script.display().to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?;

        let web_label = format!("{}.web", AUTOSTART_PREFIX);
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>{script}</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
            label = web_label,
            script = xml_escape(&web_script.display().to_string()),
            home = xml_escape(&home.display().to_string()),
            log = xml_escape(&dsh.join("launchd-web.log").display().to_string()),
        );
        fs::write(&web_plist, plist).map_err(|error| {
            log::error!("[dsh 自启(mac)] 写 plist 失败: {}", error);
            trf(
                "Failed to write {path}: {error}",
                &[
                    ("path", web_plist.display().to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?;
        unload(&format!("{}.web", AUTOSTART_PREFIX), &web_plist);
        Command::new("launchctl")
            .args(["load", "-w"])
            .arg(&web_plist)
            .output()
            .map_err(|error| {
                log::error!("[dsh 自启(mac)] 注册 launchd agent 失败: {}", error);
                trf(
                    "Cannot register launchd agent: {error}",
                    &[("error", error.to_string())],
                )
            })?;
    } else {
        unload(&format!("{}.web", AUTOSTART_PREFIX), &web_plist);
        let _ = fs::remove_file(&web_plist);
        let _ = fs::remove_file(&web_script);
    }
    Ok(())
}
#[cfg(target_os = "windows")]
fn windows_startup_dir() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"),
    )
}

#[cfg(target_os = "windows")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let dsh = dsh_dir()?;
    fs::create_dir_all(&dsh).map_err(|error| {
        log::error!("[dsh 自启(win)] 创建 ~/.dsh 目录失败: {}", error);
        trf(
            "Failed to create directory: {error}",
            &[("error", error.to_string())],
        )
    })?;
    let web_cmd = dsh.join("start-web.cmd");
    let legacy_proxy_cmd = dsh.join("start-proxy.cmd");
    let startup = windows_startup_dir()
        .ok_or_else(|| {
            let err = tr("Cannot locate the Windows Startup folder (APPDATA is missing)");
            log::error!("[dsh 自启(win)] {}", err);
            err
        })?;
    let vbs = startup.join("dsh-remote-autostart.vbs");

    let _ = fs::remove_file(&legacy_proxy_cmd);
    kill_by_pattern("loopback-proxy.js");

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let ts = tailscale_path().ok_or_else(|| tr("Tailscale is not installed"))?;
        let login = resolve_tailscale_login(&ts)?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        let trusted = if fqdn.is_empty() {
            String::new()
        } else {
            format!(" --trusted-host {}", fqdn)
        };
        let node_dir = Path::new(&node)
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let web = format!(
            "@echo off\r\nrem generated by Codex Pro Max; do not edit\r\nset \"PATH={node_dir};%PATH%\"\r\n\"{node}\" -e \"{guard}\" >nul 2>&1\r\nif %errorlevel%==0 exit /b 0\r\nset \"{login_env}={login}\"\r\ncall \"{dsh}\" --profile web --host 127.0.0.1 --port {port}{trusted}\r\n",
            node_dir = node_dir,
            node = node,
            guard = port_guard_js(WEB_PORT),
            login_env = TAILSCALE_LOGIN_ENV,
            login = login,
            dsh = dsh_bin.display(),
            port = WEB_PORT,
            trusted = trusted,
        );
        fs::write(&web_cmd, web).map_err(|error| {
            log::error!("[dsh 自启(win)] 写 start-web.cmd 失败: {}", error);
            trf(
                "Failed to write {path}: {error}",
                &[
                    ("path", web_cmd.display().to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?;
        let vbs_body = format!(
            "' generated by Codex Pro Max; do not edit\r\nSet sh = CreateObject(\"WScript.Shell\")\r\nsh.Run \"\"\"{web}\"\"\", 0, False\r\n",
            web = web_cmd.display(),
        );
        fs::create_dir_all(&startup).map_err(|error| {
            log::error!("[dsh 自启(win)] 创建启动文件夹失败: {}", error);
            trf(
                "Failed to create directory: {error}",
                &[("error", error.to_string())],
            )
        })?;
        fs::write(&vbs, vbs_body).map_err(|error| {
            log::error!("[dsh 自启(win)] 写自启 vbs 失败: {}", error);
            trf(
                "Failed to write {path}: {error}",
                &[
                    ("path", vbs.display().to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?;
    } else {
        let _ = fs::remove_file(&vbs);
        let _ = fs::remove_file(&web_cmd);
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn autostart_impl(enabled: bool) -> Result<(), String> {
    let home = config::home_dir()?;
    let units_dir = home.join(".config/systemd/user");
    let autostart_dir = home.join(".config/autostart");
    let dsh = dsh_dir()?;
    let web_script = dsh.join("start-web.sh");
    let legacy_proxy_script = dsh.join("start-proxy.sh");
    let web_unit = units_dir.join("dsh-remote-web.service");
    let legacy_proxy_unit = units_dir.join("dsh-remote-proxy.service");
    let web_desktop = autostart_dir.join("dsh-remote-web.desktop");
    let legacy_proxy_desktop = autostart_dir.join("dsh-remote-proxy.desktop");

    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", "dsh-remote-proxy.service"])
        .output();
    let _ = fs::remove_file(&legacy_proxy_unit);
    let _ = fs::remove_file(&legacy_proxy_desktop);
    let _ = fs::remove_file(&legacy_proxy_script);
    kill_by_pattern("loopback-proxy.js");

    if enabled {
        let node = resolve_node_bin()?;
        let dsh_bin = resolve_dsh_bin()?;
        let ts = tailscale_path().ok_or_else(|| tr("Tailscale is not installed"))?;
        let login = resolve_tailscale_login(&ts)?;
        let fqdn = resolve_fqdn().unwrap_or_default();
        fs::write(
            &web_script,
            render_start_web(&node, &dsh_bin.display().to_string(), &fqdn, &login),
        )
        .map_err(|error| {
            log::error!("[dsh 自启(linux)] 写启动脚本失败: {}", error);
            trf(
                "Failed to write {path}: {error}",
                &[
                    ("path", web_script.display().to_string()),
                    ("error", error.to_string()),
                ],
            )
        })?;

        if systemd_user_available() {
            fs::create_dir_all(&units_dir).map_err(|error| {
                log::error!("[dsh 自启(linux)] 创建 systemd 目录失败: {}", error);
                trf(
                    "Failed to create directory: {error}",
                    &[("error", error.to_string())],
                )
            })?;
            let unit = format!(
                "[Unit]\nDescription=DeepSeek Harness web (Tailscale authorization)\nAfter=network.target\n\n[Service]\nType=simple\nExecStart=/bin/sh {script}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
                script = sh_quote(&web_script.display().to_string()),
            );
            fs::write(&web_unit, unit).map_err(|error| {
                log::error!("[dsh 自启(linux)] 写 systemd unit 失败: {}", error);
                trf(
                    "Failed to write {path}: {error}",
                    &[
                        ("path", web_unit.display().to_string()),
                        ("error", error.to_string()),
                    ],
                )
            })?;
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .output();
            let output = Command::new("systemctl")
                .args(["--user", "enable", "dsh-remote-web.service"])
                .output()
                .map_err(|error| {
                    log::error!("[dsh 自启(linux)] 执行 systemctl enable 失败: {}", error);
                    trf(
                        "Cannot enable systemd unit: {error}",
                        &[("error", error.to_string())],
                    )
                })?;
            if !output.status.success() {
                let err = trf(
                    "Cannot enable systemd unit: {error}",
                    &[(
                        "error",
                        String::from_utf8_lossy(&output.stderr).trim().to_string(),
                    )],
                );
                log::error!("[dsh 自启(linux)] 启用 systemd 服务失败: {}", err);
                return Err(err);
            }
            let _ = fs::remove_file(&web_desktop);
        } else {
            fs::create_dir_all(&autostart_dir).map_err(|error| {
                log::error!("[dsh 自启(linux)] 创建 autostart 目录失败: {}", error);
                trf(
                    "Failed to create directory: {error}",
                    &[("error", error.to_string())],
                )
            })?;
            fs::write(
                &web_desktop,
                render_desktop_entry(
                    "DeepSeek Harness web (Tailscale authorization)",
                    &web_script,
                ),
            )
            .map_err(|error| {
                log::error!("[dsh 自启(linux)] 写 .desktop 失败: {}", error);
                trf(
                    "Failed to write {path}: {error}",
                    &[
                        ("path", web_desktop.display().to_string()),
                        ("error", error.to_string()),
                    ],
                )
            })?;
            let _ = fs::remove_file(&web_unit);
        }
    } else {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "dsh-remote-web.service"])
            .output();
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        let _ = fs::remove_file(&web_unit);
        let _ = fs::remove_file(&web_desktop);
        let _ = fs::remove_file(&web_script);
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn systemd_user_available() -> bool {
    match Command::new("systemctl")
        .args(["--user", "is-system-running"])
        .output()
    {
        Ok(o) => {
            let out = String::from_utf8_lossy(&o.stdout).to_lowercase();
            o.status.success()
                || out.contains("running")
                || out.contains("degraded")
                || out.contains("starting")
        }
        Err(_) => false,
    }
}

/// XDG autostart .desktop 文件（Linux 兜底自启机制）
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn render_desktop_entry(name: &str, script: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=DeepSeek Harness remote access via Tailscale\nExec=/bin/sh {script}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        name = name,
        script = sh_quote(&script.display().to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_version, plugin_profile_is_current, port_guard_js, render_desktop_entry,
        render_start_web, rpc_request, serve_failure_solution, serve_status_targets_web, sh_quote,
        tailscale_login_from_status_json, win_cmd_line, win_quote,
    };
    use crate::i18n::set_current;
    use std::path::Path;

    #[test]
    fn sh_quote_handles_spaces_and_quotes() {
        assert_eq!(sh_quote("/Users/a b/node"), "'/Users/a b/node'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("/usr/local/bin/node"), "'/usr/local/bin/node'");
    }

    #[test]
    fn start_scripts_embed_guard_and_exec() {
        let web = render_start_web(
            "/usr/local/bin/node",
            "/home/u/.npm-global/bin/dsh",
            "etmacmini.ts.net",
            "owner@example.com",
        );
        assert!(web.contains("net.connect(3899"));
        assert!(web.contains("export DSH_TAILSCALE_ALLOWED_LOGINS='owner@example.com'"));
        assert!(web.contains("--trusted-host 'etmacmini.ts.net'"));
        assert!(web.contains(
            "exec '/home/u/.npm-global/bin/dsh' --profile web --host 127.0.0.1 --port 3899"
        ));
        // npm shim 的 #!/usr/bin/env node 依赖 PATH：launchd/systemd 裸 PATH
        // 下必须显式补 node 所在目录，否则开机自启以 127 秒退
        assert!(web.contains("export PATH='/usr/local/bin':$PATH"));
        assert!(!web.contains("SSH_CONNECTION"));
        assert!(!web.contains("3898"));
    }

    #[test]
    fn auth_plugin_profile_requires_pinned_specs_and_bundle_entries() {
        let current = r#"{
          "dependencies": {
            "@dsh-external/dsh-client-connection-authz": "file:/opt/dsh-plugins/connection.tgz",
            "@dsh-external/dsh-auth-tailscale": "file:/opt/dsh-plugins/tailscale.tgz"
          },
          "dsh": { "profile": { "bundles": [
            "@deepseek-ai/dsh-base",
            "@deepseek-ai/dsh-web-app",
            "@dsh-external/dsh-auth-tailscale",
            "@dsh-external/dsh-client-connection-authz"
          ] } }
        }"#;
        assert!(plugin_profile_is_current(
            current,
            "file:/opt/dsh-plugins/connection.tgz",
            "file:/opt/dsh-plugins/tailscale.tgz",
        ));

        let stale = current.replace("connection.tgz", "connection-old.tgz");
        assert!(!plugin_profile_is_current(
            &stale,
            "file:/opt/dsh-plugins/connection.tgz",
            "file:/opt/dsh-plugins/tailscale.tgz",
        ));
        let missing_bundle = current.replace(
            ",\n            \"@dsh-external/dsh-client-connection-authz\"",
            "",
        );
        assert!(!plugin_profile_is_current(
            &missing_bundle,
            "file:/opt/dsh-plugins/connection.tgz",
            "file:/opt/dsh-plugins/tailscale.tgz",
        ));
    }

    #[test]
    fn tailscale_login_maps_self_user_id_exactly() {
        let status = r#"{
          "Self": { "UserID": 42 },
          "User": {
            "7": { "LoginName": "other@example.com" },
            "42": { "LoginName": "owner@example.com" }
          }
        }"#;
        assert_eq!(
            tailscale_login_from_status_json(status).unwrap(),
            "owner@example.com"
        );

        let malformed = r#"{"Self":{"UserID":42},"User":{"42":{"LoginName":"bad,login"}}}"#;
        assert!(tailscale_login_from_status_json(malformed).is_err());
        let cmd_expansion =
            r#"{"Self":{"UserID":42},"User":{"42":{"LoginName":"bad%PATH%@example.com"}}}"#;
        assert!(tailscale_login_from_status_json(cmd_expansion).is_err());
        assert!(tailscale_login_from_status_json("{}").is_err());
    }

    #[test]
    fn serve_status_matches_only_the_dsh_web_target() {
        let ready = "https://node.example.ts.net (tailnet only)\n|-- / proxy http://127.0.0.1:3899";
        assert!(serve_status_targets_web(ready));
        assert!(!serve_status_targets_web(
            "https://node.example.ts.net\n|-- / proxy http://127.0.0.1:13899",
        ));
        assert!(!serve_status_targets_web("No serve config"));
    }

    #[test]
    fn auth_start_script_binds_loopback_and_exports_allowlist() {
        let web = render_start_web(
            "/usr/local/bin/node",
            "/home/u/.npm-global/bin/dsh",
            "node.tailnet.ts.net",
            "owner@example.com",
        );
        assert!(web.contains("export DSH_TAILSCALE_ALLOWED_LOGINS='owner@example.com'"));
        assert!(web.contains("--host 127.0.0.1 --port 3899"));
        assert!(web.contains("--trusted-host 'node.tailnet.ts.net'"));
        assert!(!web.contains("SSH_CONNECTION"));
        assert!(!web.contains("3898"));
    }

    #[test]
    fn guard_js_targets_loopback() {
        assert!(port_guard_js(3899).contains("net.connect(3899,'127.0.0.1')"));
    }

    #[test]
    fn desktop_entry_quotes_script_path() {
        // XDG autostart 的 Exec 由桌面环境按 GLib 规则解析：单引号引路径
        assert_eq!(
            render_desktop_entry("DeepSeek Harness web (remote access)", Path::new("/home/u/.dsh/start-web.sh")),
            "[Desktop Entry]\nType=Application\nName=DeepSeek Harness web (remote access)\nComment=DeepSeek Harness remote access via Tailscale\nExec=/bin/sh '/home/u/.dsh/start-web.sh'\nTerminal=false\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
        );
        // 带空格路径也能被单引号包住
        assert!(
            render_desktop_entry("x", Path::new("/home/u a/.dsh/start-web.sh"))
                .contains("Exec=/bin/sh '/home/u a/.dsh/start-web.sh'")
        );
    }

    #[test]
    fn win_quote_keeps_backslashes_literal() {
        // 反斜杠是字面量：不能翻倍
        assert_eq!(win_quote(r"C:\Program Files\nodejs\node.exe"), "\"C:\\Program Files\\nodejs\\node.exe\"");
        assert_eq!(win_quote(r"C:\Windows\System32\cmd.exe"), r"C:\Windows\System32\cmd.exe");
        assert_eq!(win_quote("status"), "status");
        assert_eq!(win_quote("a\"b"), "\"a\\\"b\"");
    }

    #[test]
    fn win_cmd_line_outer_wraps_for_cmd_slash_c() {
        // cmd /c 会剥掉首尾引号，所以整体必须再包一层
        assert_eq!(
            win_cmd_line(r"C:\Program Files\Tailscale\tailscale.exe", &["status"]),
            "\"\"C:\\Program Files\\Tailscale\\tailscale.exe\" status\""
        );
        assert_eq!(
            win_cmd_line("npm", &["install", "-g", "@deepseek-ai/dsh"]),
            "\"npm install -g @deepseek-ai/dsh\""
        );
        assert_eq!(
            win_cmd_line(r"C:\Users\a b\dsh-plugins\auth.tgz", &[]),
            "\"\"C:\\Users\\a b\\dsh-plugins\\auth.tgz\"\""
        );
    }

    #[test]
    fn normalize_version_extracts_parseable_core() {
        // dsh 实测输出（无 v 前缀）
        assert_eq!(normalize_version("0.1.0-rc.6"), "0.1.0-rc.6");
        // v 前缀被剥掉
        assert_eq!(normalize_version("v0.1.0-rc.6"), "0.1.0-rc.6");
        // 带前缀/尾缀杂质也能提取
        assert_eq!(normalize_version("dsh 0.1.0-rc.6"), "0.1.0-rc.6");
        assert_eq!(normalize_version("0.1.0-rc.6 (build abc)"), "0.1.0-rc.6");
        // 无法解析时回退原串（兼容性检查会明确判定不匹配）
        assert_eq!(normalize_version("garbage"), "garbage");
        assert_eq!(normalize_version(""), "");
    }

    #[test]
    fn windows_cmd_scripts_use_call_for_dsh() {
        // 回归：cmd 对以引号开头的命令行会剥掉首尾引号，dsh 路径含空格时
        // 直接执行会拆碎；.cmd 内必须用 call 前缀。Windows autostart_impl 里
        // 的 web 脚本模板以 `call "{dsh}"` 起行——用等价的最小复现断言该形态
        let dsh_path = r"C:\Users\a b\.npm-global\dsh.cmd";
        let line = format!(
            "call \"{}\" --profile web --host 127.0.0.1 --port 3899",
            dsh_path
        );
        assert!(line.starts_with("call \""));
    }

    #[test]
    fn ws_probe_targets_events_host() {
        // WS 探测脚本：发真实 upgrade 握手（curl 的 HTTP/2 假 426 不适用），
        // 拿到 HTTP/1.1 101 即成功；net/tls 双路径，不依赖 Node v22+ 内置
        // WebSocket——Node 18+ 都能跑
        assert!(super::WS_PROBE_JS.contains("HTTP/1.1 101"));
        assert!(super::WS_PROBE_JS.contains("Sec-WebSocket-Key"));
        assert!(super::WS_PROBE_JS.contains("net.connect"));
        assert!(super::WS_PROBE_JS.contains("tls.connect"));
        // 101 → exit 0（成功）；其余状态/错误/超时 → exit 1
        assert!(super::WS_PROBE_JS.contains("?0:1"));
        assert!(super::WS_PROBE_JS.contains("finish(1)"));
        assert!(super::WS_PROBE_JS.contains("process.exit(c)"));
        // 脚本不含双引号：Windows cmd /c 引号转义安全（含双引号会拆碎 -e 参数）
        assert!(!super::WS_PROBE_JS.contains('"'));
    }

    #[test]
    fn ws_url_rewrites_https_to_wss() {
        // ws_endpoint_ok 的 URL 改写：https:// → wss://，拼 /api/events.host
        let url = "https://etmacmini.taildde4.ts.net";
        let ws_url = format!("{}/api/events.host", url.replacen("https://", "wss://", 1));
        assert_eq!(ws_url, "wss://etmacmini.taildde4.ts.net/api/events.host");
    }

    #[test]
    fn rpc_request_is_loopback_json_post() {
        // 敏感 API 校验请求：Host 为 loopback、无 Origin、JSON body 与
        // Content-Length 一致。
        let req = rpc_request("settings.describe");
        assert!(req.starts_with("POST /api/settings.describe HTTP/1.1\r\n"));
        assert!(req.contains("Host: 127.0.0.1"));
        assert!(req.contains("Content-Type: application/json"));
        assert!(!req.contains("Origin:"));
        let body =
            r#"{"type":"client-request","rpcId":"t1","method":"settings.describe","payload":{}}"#;
        assert!(req.contains(body));
        assert!(req.contains(&format!("Content-Length: {}\r\n", body.len())));
    }

    #[test]
    fn serve_failure_solution_branches_on_tls_hint() {
        // 教程 3.3：HTTPS Certificates 是与 MagicDNS 独立的开关；serve 报
        // TLS 证书类错误时方案指向 admin/dns，其余才指向 serve 授权链接
        set_current("en");
        let tls_err = "500 Internal Server Error: your Tailscale account does not support getting TLS certs";
        assert_eq!(
            serve_failure_solution(tls_err),
            "MagicDNS or HTTPS Certificates may not be enabled; open https://login.tailscale.com/admin/dns and enable MagicDNS and HTTPS Certificates, then retry"
        );
        let serve_err = "Serve is not enabled on your tailnet. To enable Serve, visit: https://login.tailscale.com/f/serve?node=abc";
        assert_eq!(
            serve_failure_solution(serve_err),
            "Open the authorization link in the error output to enable Serve for this tailnet (https://login.tailscale.com/f/serve), then retry"
        );
        set_current("en");
    }
}
