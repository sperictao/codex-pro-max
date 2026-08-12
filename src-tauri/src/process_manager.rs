use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use std::process::Stdio;

use crate::i18n::{tr, trf};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// 错误前缀标记：Codex 已在运行但未开 CDP（Windows 与 macOS 完整模式发出）。
/// 前端据此弹窗询问是否重启 Codex，其余错误照常 toast
#[allow(dead_code)] // Linux 构建无此流程
pub const CODEX_RUNNING_NO_CDP_MARK: &str = "CODEX_RUNNING_NO_CDP|";

/// 枚举桌面版 Codex/ChatGPT 进程 PID（仅 Windows）
/// codex.exe 必须按路径复核（CLI 同名，在 npm 全局目录，误杀会毁掉用户 CLI 会话）；
/// chatgpt.exe 无同名 CLI，按名匹配即可
#[cfg(target_os = "windows")]
fn codex_processes() -> Vec<u32> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let mut pids = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return pids };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if is_desktop_codex(&name, entry.th32ProcessID) {
                    pids.push(entry.th32ProcessID);
                }
                if Process32NextW(snap, &mut entry).is_err() { break; }
            }
        }
        let _ = CloseHandle(snap);
    }
    pids
}

#[cfg(target_os = "windows")]
fn is_desktop_codex(exe_name: &str, pid: u32) -> bool {
    let name = exe_name.to_lowercase();
    if name == "chatgpt.exe" {
        return true;
    }
    if name == "codex.exe" {
        return process_path(pid)
            .map(|p| {
                let p = p.to_lowercase();
                p.contains("\\openai\\codex\\") || p.contains("\\windowsapps\\openai.")
            })
            .unwrap_or(false);
    }
    false
}

/// 查进程完整路径；无权访问（系统进程等）时返回 None，调用方按非目标处理
#[cfg(target_os = "windows")]
fn process_path(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            h,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// 关闭正在运行的桌面版 Codex：先 WM_CLOSE 优雅退出，10 秒未尽则强杀
/// （非 Windows 平台不会被调用——标记只在 Windows 发出——空实现保底）
#[cfg(target_os = "windows")]
pub async fn quit_codex() -> Result<(), String> {
    for pid in codex_processes() {
        // 不带 /F：GUI 主进程收到 WM_CLOSE 优雅退出；无窗口子进程会被拒，随主进程消亡
        let mut cmd = Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T"]);
        setup_no_window(&mut cmd);
        let _ = cmd.output().await;
    }
    for _ in 0..20 {
        if codex_processes().is_empty() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    for pid in codex_processes() {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/F", "/PID", &pid.to_string(), "/T"]);
        setup_no_window(&mut cmd);
        let _ = cmd.output().await;
    }
    for _ in 0..6 {
        if codex_processes().is_empty() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(tr("Codex did not exit in time; please quit it manually and retry"))
}

/// macOS 桌面版是否运行：pgrep 大小写敏感，"Codex" 不会误匹配 CLI 的 codex
#[cfg(target_os = "macos")]
fn codex_running() -> bool {
    ["ChatGPT", "Codex"].iter().any(|name| {
        std::process::Command::new("pgrep")
            .args(["-x", *name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// macOS：osascript quit 优雅退出（首次会弹「允许控制」授权，拒绝则走 pkill 兜底）
#[cfg(target_os = "macos")]
pub async fn quit_codex() -> Result<(), String> {
    if !codex_running() {
        return Ok(());
    }
    for app in ["ChatGPT", "Codex"] {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("quit app \"{}\"", app)])
            .output();
    }
    for _ in 0..20 {
        if !codex_running() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    for name in ["ChatGPT", "Codex"] {
        let _ = std::process::Command::new("pkill").args(["-x", name]).output();
    }
    for _ in 0..6 {
        if !codex_running() {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(tr("Codex did not exit in time; please quit it manually and retry"))
}

/// 其他平台不发出标记、不会被调用，空实现保底
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub async fn quit_codex() -> Result<(), String> {
    Ok(())
}

/// 为 Command 添加跨平台无窗口设置
/// Windows: CREATE_NO_WINDOW 防止弹出终端窗口
/// Unix: 进程组分离
fn setup_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        // tokio::process::Command 自带同名 inherent 方法，无需导入 std 的 CommandExt
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        // Unix 上 tokio::process::Command 默认会继承进程组
        // 设置 process_group(0) 创建新进程组，使子进程独立
        cmd.process_group(0);
    }
}

/// 进程状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

/// 进程信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub name: String,
    pub status: ProcessStatus,
    pub pid: Option<u32>,
    pub message: String,
}

/// 管理的单个子进程
pub struct ManagedProcess {
    pub name: String,
    pub child: Option<Child>,
    pub status: ProcessStatus,
    pub message: String,
    /// 子进程 stdout/stderr 尾部（环形缓冲），把启动失败原因暴露给 UI
    pub output_tail: Arc<std::sync::Mutex<String>>,
}

impl ManagedProcess {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            child: None,
            status: ProcessStatus::Stopped,
            message: String::new(),
            output_tail: Arc::new(std::sync::Mutex::new(String::new())),
        }
    }

    pub fn info(&self) -> ProcessInfo {
        ProcessInfo {
            name: self.name.clone(),
            status: self.status.clone(),
            pid: self.child.as_ref().and_then(|c| c.id()),
            message: self.message.clone(),
        }
    }
}

/// 进程管理器，管理 taskboard 服务和 codex 注入器
pub struct ProcessManager {
    pub taskboard: Arc<Mutex<ManagedProcess>>,
    pub injector: Arc<Mutex<ManagedProcess>>,
}

/// 解析 node 可执行文件路径
/// ponytail: GUI 应用（Finder 启动）PATH 只有 /usr/bin:/bin，裸 "node" 会 ENOENT；
/// 空配置时先探测常见安装位置，探测不到再退回 PATH 查找
pub fn resolve_node(node_path: &str) -> String {
    if !node_path.is_empty() {
        return node_path.to_string();
    }
    #[cfg(unix)]
    {
        let mut candidates = vec![
            "/opt/homebrew/bin/node".to_string(),
            "/usr/local/bin/node".to_string(),
        ];
        if let Ok(home) = std::env::var("HOME") {
            candidates.push(format!("{}/.local/bin/node", home));
        }
        candidates.push("/usr/bin/node".to_string());
        for c in candidates {
            if std::path::Path::new(&c).exists() {
                return c;
            }
        }
    }
    "node".to_string()
}

fn tail_of(buf: &Arc<std::sync::Mutex<String>>) -> String {
    let Ok(b) = buf.lock() else { return String::new() };
    let lines: Vec<&str> = b.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" | ")
}

/// 持续排空子进程 stdout/stderr 到尾部缓冲
/// 必须排空：管道无人读取，写满 64KB 后子进程阻塞（watch 模式注入器持续打日志）
fn spawn_output_drain<R>(mut reader: R, buf: Arc<std::sync::Mutex<String>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::AsyncReadExt;
    tokio::spawn(async move {
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&chunk[..n]);
                    let Ok(mut b) = buf.lock() else { break };
                    b.push_str(&text);
                    let excess = b.len().saturating_sub(4000);
                    if excess > 0 {
                        let mut idx = excess;
                        while !b.is_char_boundary(idx) {
                            idx += 1;
                        }
                        b.drain(..idx);
                    }
                }
            }
        }
    });
}

fn drain_child_output(child: &mut Child, buf: &Arc<std::sync::Mutex<String>>) {
    if let Some(stdout) = child.stdout.take() {
        spawn_output_drain(stdout, buf.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_output_drain(stderr, buf.clone());
    }
}

/// spawn 成功不代表存活：注入器大量失败路径（CDP 不可达、端口占用、
/// 脚本报错）都会在 1 秒内退出。宽限检查后把退出码和日志尾部抛给 UI，
/// 不再让"运行中"掩盖秒退
async fn fail_if_exited(proc: &mut ManagedProcess) -> Result<(), String> {
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let Some(child) = proc.child.as_mut() else { return Ok(()) };
    match child.try_wait() {
        Ok(Some(status)) => {
            let tail = tail_of(&proc.output_tail);
            let detail = if tail.is_empty() { String::new() } else { format!(": {}", tail) };
            proc.child = None;
            proc.status = ProcessStatus::Failed;
            proc.message = trf("Exited immediately after start ({status}){detail}", &[
                ("status", status.to_string()),
                ("detail", detail),
            ]);
            crate::notify_process_failure(&proc.name, &proc.message);
            Err(proc.message.clone())
        }
        _ => Ok(()),
    }
}

/// 轮询时做活性检测：子进程意外退出（哪怕启动很久之后）要翻成 Failed，
/// 否则 UI 永远停在"运行中"
fn refresh_liveness(proc: &mut ManagedProcess) {
    if proc.status != ProcessStatus::Running && proc.status != ProcessStatus::Starting {
        return;
    }
    let Some(child) = proc.child.as_mut() else { return };
    if let Ok(Some(status)) = child.try_wait() {
        let tail = tail_of(&proc.output_tail);
        let detail = if tail.is_empty() { String::new() } else { format!(": {}", tail) };
        proc.child = None;
        proc.status = ProcessStatus::Failed;
        proc.message = trf("Process exited unexpectedly ({status}){detail}", &[
            ("status", status.to_string()),
            ("detail", detail),
        ]);
        crate::notify_process_failure(&proc.name, &proc.message);
    }
}

fn cdp_reachable(port: u16) -> bool {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        std::time::Duration::from_millis(500),
    )
    .is_ok()
}

/// token 模式 health 探测的 HMAC-SHA256 proof（与 server/injector 算法一致）
fn hmac_proof(secret: &str, challenge: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(challenge.as_bytes());
    format!("{:x}", mac.finalize().into_bytes())
}

/// 端口上已有健康的 Taskboard 服务时直接复用；重复 spawn 只会 EADDRINUSE。
/// taskboard v0.2.3 token 模式下 /health 必须带 challenge 头，返回的 proof 须等于
/// HMAC(secret, challenge) 才算健康——裸 200 是旧版判定，token 模式会误判为「不可达」
/// 导致注入器再起一个 server 抢端口
async fn taskboard_health_reachable(host: &str, port: u16, secret: &str) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let timeout = std::time::Duration::from_millis(800);
    let Ok(Ok(mut stream)) = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect((host, port)),
    ).await else { return false };
    // server 要求 challenge 为 32~128 位 hex，uuid.simple() 恰好 32 位
    let challenge = uuid::Uuid::new_v4().simple().to_string();
    let req = format!(
        "GET /health HTTP/1.1\r\nHost: {host}:{port}\r\nx-codex-taskboard-challenge: {challenge}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(req.as_bytes()).await.is_err() { return false; }
    let mut buf = [0u8; 4096];
    let Ok(Ok(n)) = tokio::time::timeout(timeout, stream.read(&mut buf)).await else { return false };
    let head = String::from_utf8_lossy(&buf[..n]);
    if !(head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")) {
        return false;
    }
    // 非 token 模式（旧版/未传 secret 的残留服务）返回 {status:"ok"} 无 proof，
    // 与已配置 token 不一致，判为不可达
    let body = head.split("\r\n\r\n").nth(1).unwrap_or_default();
    let proof_ok = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("proof").and_then(|p| p.as_str()).map(String::from))
        .map(|proof| proof == hmac_proof(secret, &challenge))
        .unwrap_or(false);
    proof_ok
}

/// 确保有一个带 CDP 调试端口的 Codex 实例，没有就拉起并等端口就绪
/// new_instance=true（独立窗口模式）：macOS 用 open -n 强制新实例，不动现有窗口
/// new_instance=false（完整模式）：拉起主实例；若已在运行且无 CDP，
/// 调试参数不会生效，等待超时后提示用户先退出 Codex
async fn ensure_codex_cdp(app_path: &str, port: u16, new_instance: bool) -> Result<(), String> {
    if cdp_reachable(port) {
        return Ok(());
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        // Windows 无法强制新实例；macOS 仅完整模式（open 不带 -n）有此问题——
        // 已在运行的实例会吞掉调试参数，拉起也白等。立即返回带标记的错误，
        // 由前端弹窗询问是否关闭并重启 Codex
        #[cfg(target_os = "windows")]
        let running = !codex_processes().is_empty();
        #[cfg(target_os = "macos")]
        let running = !new_instance && codex_running();
        if running {
            return Err(format!(
                "{}{}",
                CODEX_RUNNING_NO_CDP_MARK,
                tr("Codex is already running without the CDP debug port")
            ));
        }
    }
    if app_path.is_empty() {
        return Err(trf(
            "CDP port {port} is not responding and no Codex app path is configured. Select the Codex app in Settings, or start Codex manually with --remote-debugging-port={port}",
            &[("port", port.to_string())],
        ));
    }
    let debug_args = [
        format!("--remote-debugging-port={}", port),
        format!("--remote-allow-origins=http://127.0.0.1:{}", port),
    ];
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("/usr/bin/open");
        if new_instance {
            cmd.arg("-n");
        }
        cmd.arg("-a").arg(app_path).arg("--args").args(&debug_args);
        let out = cmd.output().map_err(|e| trf("Cannot launch Codex: {error}", &[("error", e.to_string())]))?;
        if !out.status.success() {
            return Err(trf(
                "Failed to launch Codex: {error}",
                &[("error", String::from_utf8_lossy(&out.stderr).trim().to_string())],
            ));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = new_instance;
        // 商店版（msix: 哨兵）：COM 激活并把调试参数透传到应用命令行
        #[cfg(target_os = "windows")]
        if let Some(amid) = app_path.strip_prefix("msix:") {
            crate::launch_store_app(amid, &debug_args.join(" "))?;
        } else {
            std::process::Command::new(app_path)
                .args(&debug_args)
                .spawn()
                .map_err(|e| trf("Cannot launch Codex ({path}): {error}", &[
                    ("path", app_path.to_string()),
                    ("error", e.to_string()),
                ]))?;
        }
        // Linux 直接带参数拉起 exe
        #[cfg(not(target_os = "windows"))]
        std::process::Command::new(app_path)
            .args(&debug_args)
            .spawn()
            .map_err(|e| trf("Cannot launch Codex ({path}): {error}", &[
                ("path", app_path.to_string()),
                ("error", e.to_string()),
            ]))?;
    }
    // 等窗口和 CDP 就绪，最多 15 秒
    for _ in 0..30 {
        if cdp_reachable(port) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    // Windows 上 new_instance 被忽略（无法强制新实例），超时多半意味着 Codex 已在
    // 运行且没带调试参数——单实例激活会丢弃参数，故两种模式给出一致提示
    Err(if new_instance && cfg!(target_os = "macos") {
        trf("Timed out waiting for Codex CDP port {port} to be ready", &[("port", port.to_string())])
    } else {
        trf(
            "Timed out waiting for Codex CDP port {port} to be ready. If Codex is already running, quit it completely and retry",
            &[("port", port.to_string())],
        )
    })
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            taskboard: Arc::new(Mutex::new(ManagedProcess::new("taskboard-server"))),
            injector: Arc::new(Mutex::new(ManagedProcess::new("codex-injector"))),
        }
    }

    /// taskboard 是否已被本管理器置为运行态
    /// （一键启动重试路径靠它幂等跳过，避免「已在运行」中断注入器启动）
    pub async fn taskboard_is_running(&self) -> bool {
        self.taskboard.lock().await.status == ProcessStatus::Running
    }

    /// 启动 taskboard 服务（token 模式：与注入器共用 instance_token/instance_secret env）
    pub async fn start_taskboard(
        &self,
        taskboard_path: &str,
        node_path: &str,
        host: &str,
        port: u16,
        instance_token: &str,
        instance_secret: &str,
    ) -> Result<(), String> {
        let mut tb = self.taskboard.lock().await;
        if tb.status == ProcessStatus::Running || tb.status == ProcessStatus::Starting {
            return Err(tr("Taskboard server is already running"));
        }

        // 残留/外部实例健康时直接复用，避免 EADDRINUSE 秒退；
        // child 置空，停止时不会去杀不属于自己的进程
        if taskboard_health_reachable(host, port, instance_secret).await {
            tb.child = None;
            tb.status = ProcessStatus::Running;
            tb.message = trf("Reusing Taskboard server already running at http://{host}:{port}", &[
                ("host", host.to_string()),
                ("port", port.to_string()),
            ]);
            return Ok(());
        }

        tb.status = ProcessStatus::Starting;
        tb.message = tr("Starting Taskboard server...");

        let node = resolve_node(node_path);
        let server_script = format!("{}/server/index.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&server_script);
        cmd.env("CODEX_TASKBOARD_HOST", host);
        cmd.env("CODEX_TASKBOARD_PORT", port.to_string());
        cmd.env("CODEX_TASKBOARD_INSTANCE_TOKEN", instance_token);
        cmd.env("CODEX_TASKBOARD_INSTANCE_SECRET", instance_secret);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(taskboard_path);
        setup_no_window(&mut cmd);

        match cmd.spawn() {
            Ok(mut child) => {
                drain_child_output(&mut child, &tb.output_tail);
                tb.child = Some(child);
                fail_if_exited(&mut tb).await?;
                tb.status = ProcessStatus::Running;
                tb.message = trf("Taskboard running at http://{host}:{port}", &[
                    ("host", host.to_string()),
                    ("port", port.to_string()),
                ]);
                Ok(())
            }
            Err(e) => {
                tb.status = ProcessStatus::Failed;
                tb.message = trf("Launch failed: {error}", &[("error", e.to_string())]);
                crate::notify_process_failure(&tb.name, &tb.message);
                Err(trf("Failed to start Taskboard server: {error}", &[("error", e.to_string())]))
            }
        }
    }

    /// 停止 taskboard 服务
    pub async fn stop_taskboard(&self) -> Result<(), String> {
        let mut tb = self.taskboard.lock().await;
        if tb.status != ProcessStatus::Running {
            return Ok(());
        }
        tb.status = ProcessStatus::Stopping;
        tb.message = tr("Stopping...");

        if let Some(child) = tb.child.as_mut() {
            // 先尝试优雅关闭
            let pid = child.id();
            if let Some(pid) = pid {
                #[cfg(unix)]
                {
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .output();
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                }
            }
            // 等待退出，超时则强制 kill
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                child.wait(),
            ).await {
                Ok(_) => {}
                Err(_) => {
                    if let Some(child) = tb.child.as_mut() {
                        let _ = child.start_kill();
                    }
                }
            }
        }

        tb.child = None;
        tb.status = ProcessStatus::Stopped;
        tb.message = tr("Stopped");
        Ok(())
    }

    /// 启动 codex 注入器
    /// 两种模式统一走跨平台的 --watch：由启动器负责拉起带 CDP 端口的
    /// Codex 实例（ensure_codex_cdp），不再用注入器的 --launch
    /// （其内部 open/pgrep/ps 仅支持 macOS，Windows 上必然失败）。
    /// token 模式：注入器从 env 读 instance token/secret，与 server 一致，
    /// 否则注入器会自生成一套导致与 launcher 起的 server 互相不可达
    pub async fn start_injector(
        &self,
        taskboard_path: &str,
        node_path: &str,
        cdp_port: u16,
        codex_app_path: &str,
        separate_window: bool,
        taskboard_port: u16,
        instance_token: &str,
        instance_secret: &str,
    ) -> Result<(), String> {
        let mut inj = self.injector.lock().await;
        if inj.status == ProcessStatus::Running || inj.status == ProcessStatus::Starting {
            return Err(tr("Codex injector is already running"));
        }

        inj.status = ProcessStatus::Starting;
        inj.message = tr("Waiting for Codex debug port...");

        if let Err(e) = ensure_codex_cdp(codex_app_path, cdp_port, separate_window).await {
            inj.status = ProcessStatus::Failed;
            // 标记只给前端识别用，状态栏与通知里剥掉
            inj.message = e.strip_prefix(CODEX_RUNNING_NO_CDP_MARK).unwrap_or(&e).to_string();
            crate::notify_process_failure(&inj.name, &inj.message);
            return Err(e);
        }

        inj.message = tr("Starting Codex injector...");

        let node = resolve_node(node_path);
        let injector_script = format!("{}/scripts/codex-injector.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&injector_script);
        cmd.arg("--watch");
        cmd.arg("--open");
        cmd.arg("--port").arg(cdp_port.to_string());

        // 注入器按 CODEX_TASKBOARD_PORT 推导 taskboard 地址（resolvePort），
        // 不传则退回默认 47823，与启动器实际端口不一致时 iframe 永远加载失败。
        // 注意：注入器不读 CODEX_TASKBOARD_APP_PATH，app 路径只用于上面的拉起。
        // instance token/secret 与 server 保持一致，注入器才认 server 为「自己的实例」
        cmd.env("CODEX_TASKBOARD_HOST", "127.0.0.1");
        cmd.env("CODEX_TASKBOARD_PORT", taskboard_port.to_string());
        cmd.env("CODEX_TASKBOARD_INSTANCE_TOKEN", instance_token);
        cmd.env("CODEX_TASKBOARD_INSTANCE_SECRET", instance_secret);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(taskboard_path);
        setup_no_window(&mut cmd);

        match cmd.spawn() {
            Ok(mut child) => {
                drain_child_output(&mut child, &inj.output_tail);
                inj.child = Some(child);
                fail_if_exited(&mut inj).await?;
                inj.status = ProcessStatus::Running;
                if separate_window {
                    inj.message = trf("Injector running (separate window, CDP port {port})", &[("port", cdp_port.to_string())]);
                } else {
                    inj.message = trf("Injector running (full launch, CDP port {port})", &[("port", cdp_port.to_string())]);
                }
                Ok(())
            }
            Err(e) => {
                inj.status = ProcessStatus::Failed;
                inj.message = trf("Launch failed: {error}", &[("error", e.to_string())]);
                crate::notify_process_failure(&inj.name, &inj.message);
                Err(trf("Failed to start Codex injector: {error}", &[("error", e.to_string())]))
            }
        }
    }

    /// 停止 codex 注入器
    pub async fn stop_injector(&self) -> Result<(), String> {
        let mut inj = self.injector.lock().await;
        if inj.status != ProcessStatus::Running {
            return Ok(());
        }
        inj.status = ProcessStatus::Stopping;
        inj.message = tr("Stopping...");

        if let Some(child) = inj.child.as_mut() {
            let pid = child.id();
            if let Some(pid) = pid {
                #[cfg(unix)]
                {
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .output();
                }
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x08000000;
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                }
            }
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                child.wait(),
            ).await {
                Ok(_) => {}
                Err(_) => {
                    if let Some(child) = inj.child.as_mut() {
                        let _ = child.start_kill();
                    }
                }
            }
        }

        inj.child = None;
        inj.status = ProcessStatus::Stopped;
        inj.message = tr("Stopped");
        Ok(())
    }

    /// 获取所有进程信息（顺带做活性检测，把秒退/意外退出翻成 Failed）
    pub async fn get_all_status(&self) -> Vec<ProcessInfo> {
        let mut tb = self.taskboard.lock().await;
        let mut inj = self.injector.lock().await;
        refresh_liveness(&mut tb);
        refresh_liveness(&mut inj);
        vec![tb.info(), inj.info()]
    }

    /// 停止所有进程
    pub async fn stop_all(&self) -> Result<(), String> {
        self.stop_injector().await?;
        self.stop_taskboard().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{hmac_proof, resolve_node};

    #[test]
    fn resolves_existing_node() {
        // 显式路径原样返回
        assert_eq!(resolve_node("/custom/node"), "/custom/node");
        // 空配置：探测到的路径必须真实存在（或退回 PATH 查找）
        let node = resolve_node("");
        if node != "node" {
            assert!(std::path::Path::new(&node).exists(), "探测结果不存在: {}", node);
        }
    }

    #[test]
    fn health_proof_matches_node_hmac() {
        // 实测向量：taskboard v0.2.3 server 在相同输入下返回同样的 proof
        //（challenge 为 32 位 hex，secret 为 64 位 hex）
        let proof = hmac_proof(
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "0123456789abcdef0123456789abcdef",
        );
        assert_eq!(
            proof,
            "da1acb03bb1d5fe42165f2ec004a20c466fc7d84e73a397c73431d1b9b2762ed"
        );
    }
}
