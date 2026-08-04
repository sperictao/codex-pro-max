use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use std::process::Stdio;

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
}

impl ManagedProcess {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            child: None,
            status: ProcessStatus::Stopped,
            message: String::new(),
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

impl ProcessManager {
    pub fn new() -> Self {
        Self {
            taskboard: Arc::new(Mutex::new(ManagedProcess::new("taskboard-server"))),
            injector: Arc::new(Mutex::new(ManagedProcess::new("codex-injector"))),
        }
    }

    /// 解析 node 可执行文件路径
    fn resolve_node(node_path: &str) -> String {
        if node_path.is_empty() {
            "node".to_string()
        } else {
            node_path.to_string()
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

        let node = Self::resolve_node(node_path);
        let server_script = format!("{}/server/index.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&server_script);
        cmd.env("CODEX_TASKBOARD_HOST", host);
        cmd.env("CODEX_TASKBOARD_PORT", port.to_string());
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(taskboard_path);

        match cmd.spawn() {
            Ok(child) => {
                tb.child = Some(child);
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
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
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
    pub async fn start_injector(
        &self,
        taskboard_path: &str,
        node_path: &str,
        cdp_port: u16,
        codex_app_path: &str,
        separate_window: bool,
    ) -> Result<(), String> {
        let mut inj = self.injector.lock().await;
        if inj.status == ProcessStatus::Running || inj.status == ProcessStatus::Starting {
            return Err("Codex 注入器已在运行".to_string());
        }

        inj.status = ProcessStatus::Starting;
        inj.message = "正在启动 Codex 注入器...".to_string();

        let node = Self::resolve_node(node_path);
        let injector_script = format!("{}/scripts/codex-injector.mjs", taskboard_path);

        let mut cmd = Command::new(&node);
        cmd.arg(&injector_script);

        if separate_window {
            // 独立窗口模式：仅注入，不启动 Codex
            cmd.arg("--watch");
            cmd.arg("--port").arg(cdp_port.to_string());
        } else {
            // 完整启动模式：启动 Codex + 注入 + 监控
            cmd.arg("--launch");
            cmd.arg("--watch");
            cmd.arg("--open");
        }

        cmd.env("CODEX_TASKBOARD_HOST", "127.0.0.1");
        cmd.env("CODEX_TASKBOARD_APP_PATH", codex_app_path);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(taskboard_path);

        match cmd.spawn() {
            Ok(child) => {
                inj.child = Some(child);
                inj.status = ProcessStatus::Running;
                if separate_window {
                    inj.message = format!("注入器运行中 (CDP 端口 {})", cdp_port);
                } else {
                    inj.message = "注入器运行中 (完整启动模式)".to_string();
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
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
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

    /// 获取所有进程信息
    pub async fn get_all_status(&self) -> Vec<ProcessInfo> {
        let tb = self.taskboard.lock().await;
        let inj = self.injector.lock().await;
        vec![tb.info(), inj.info()]
    }

    /// 停止所有进程
    pub async fn stop_all(&self) -> Result<(), String> {
        self.stop_injector().await?;
        self.stop_taskboard().await?;
        Ok(())
    }
}
