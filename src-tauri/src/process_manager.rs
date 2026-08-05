use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use std::process::Stdio;

/// 为 Command 添加跨平台无窗口设置
/// Windows: CREATE_NO_WINDOW 防止弹出终端窗口
/// Unix: 进程组分离
fn setup_no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
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
            proc.message = format!("启动后立即退出 ({}){}", status, detail);
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
        proc.message = format!("进程意外退出 ({}){}", status, detail);
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

/// 确保有一个带 CDP 调试端口的 Codex 实例，没有就拉起并等端口就绪
/// new_instance=true（独立窗口模式）：macOS 用 open -n 强制新实例，不动现有窗口
/// new_instance=false（完整模式）：拉起主实例；若已在运行且无 CDP，
/// 调试参数不会生效，等待超时后提示用户先退出 Codex
async fn ensure_codex_cdp(app_path: &str, port: u16, new_instance: bool) -> Result<(), String> {
    if cdp_reachable(port) {
        return Ok(());
    }
    if app_path.is_empty() {
        return Err(format!(
            "CDP 端口 {} 无响应，且未配置 Codex 应用路径。请在设置中选择 Codex 应用，或手动以 --remote-debugging-port={} 启动 Codex",
            port, port
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
        let out = cmd.output().map_err(|e| format!("无法启动 Codex: {}", e))?;
        if !out.status.success() {
            return Err(format!(
                "启动 Codex 失败: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows/Linux 直接带参数拉起 exe（GUI 应用，无控制台窗口问题）
        let _ = new_instance;
        std::process::Command::new(app_path)
            .args(&debug_args)
            .spawn()
            .map_err(|e| format!("无法启动 Codex ({}): {}", app_path, e))?;
    }
    // 等窗口和 CDP 就绪，最多 15 秒
    for _ in 0..30 {
        if cdp_reachable(port) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(if new_instance {
        format!("等待 Codex CDP 端口 {} 就绪超时", port)
    } else {
        format!(
            "等待 Codex CDP 端口 {} 就绪超时。若 Codex 已在运行，请先完全退出再使用完整启动模式",
            port
        )
    })
}

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            taskboard: Arc::new(Mutex::new(ManagedProcess::new("taskboard-server"))),
            injector: Arc::new(Mutex::new(ManagedProcess::new("codex-injector"))),
        }
    }

    /// 启动 taskboard 服务
    pub async fn start_taskboard(
        &self,
        taskboard_path: &str,
        node_path: &str,
        host: &str,
        port: u16,
    ) -> Result<(), String> {
        let mut tb = self.taskboard.lock().await;
        if tb.status == ProcessStatus::Running || tb.status == ProcessStatus::Starting {
            return Err("Taskboard 服务已在运行".to_string());
        }

        tb.status = ProcessStatus::Starting;
        tb.message = "正在启动 Taskboard 服务...".to_string();

        let node = resolve_node(node_path);
        let server_script = format!("{}/server/index.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&server_script);
        cmd.env("CODEX_TASKBOARD_HOST", host);
        cmd.env("CODEX_TASKBOARD_PORT", port.to_string());
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
                tb.message = format!("Taskboard 运行在 http://{}:{}", host, port);
                Ok(())
            }
            Err(e) => {
                tb.status = ProcessStatus::Failed;
                tb.message = format!("启动失败: {}", e);
                Err(format!("启动 Taskboard 服务失败: {}", e))
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
        tb.message = "正在停止...".to_string();

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
        tb.message = "已停止".to_string();
        Ok(())
    }

    /// 启动 codex 注入器
    /// 两种模式统一走跨平台的 --watch：由启动器负责拉起带 CDP 端口的
    /// Codex 实例（ensure_codex_cdp），不再用注入器的 --launch
    /// （其内部 open/pgrep/ps 仅支持 macOS，Windows 上必然失败）
    pub async fn start_injector(
        &self,
        taskboard_path: &str,
        node_path: &str,
        cdp_port: u16,
        codex_app_path: &str,
        separate_window: bool,
        taskboard_port: u16,
    ) -> Result<(), String> {
        let mut inj = self.injector.lock().await;
        if inj.status == ProcessStatus::Running || inj.status == ProcessStatus::Starting {
            return Err("Codex 注入器已在运行".to_string());
        }

        inj.status = ProcessStatus::Starting;
        inj.message = "正在等待 Codex 调试端口...".to_string();

        if let Err(e) = ensure_codex_cdp(codex_app_path, cdp_port, separate_window).await {
            inj.status = ProcessStatus::Failed;
            inj.message = e.clone();
            return Err(e);
        }

        inj.message = "正在启动 Codex 注入器...".to_string();

        let node = resolve_node(node_path);
        let injector_script = format!("{}/scripts/codex-injector.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&injector_script);
        cmd.arg("--watch");
        cmd.arg("--open");
        cmd.arg("--port").arg(cdp_port.to_string());

        // 注入器按 CODEX_TASKBOARD_PORT 推导 taskboard 地址（resolvePort），
        // 不传则退回默认 47823，与启动器实际端口不一致时 iframe 永远加载失败。
        // 注意：注入器不读 CODEX_TASKBOARD_APP_PATH，app 路径只用于上面的拉起
        cmd.env("CODEX_TASKBOARD_HOST", "127.0.0.1");
        cmd.env("CODEX_TASKBOARD_PORT", taskboard_port.to_string());
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
                    inj.message = format!("注入器运行中 (独立窗口, CDP 端口 {})", cdp_port);
                } else {
                    inj.message = format!("注入器运行中 (完整启动, CDP 端口 {})", cdp_port);
                }
                Ok(())
            }
            Err(e) => {
                inj.status = ProcessStatus::Failed;
                inj.message = format!("启动失败: {}", e);
                Err(format!("启动 Codex 注入器失败: {}", e))
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
        inj.message = "正在停止...".to_string();

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
        inj.message = "已停止".to_string();
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
    use super::resolve_node;

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
}
