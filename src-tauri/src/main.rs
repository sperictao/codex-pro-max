use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use serde::Serialize;
use tauri_plugin_updater::UpdaterExt;

mod config;
mod process_manager;

use config::LauncherConfig;
use process_manager::{ProcessManager, ProcessInfo};

/// 应用共享状态
pub struct AppState {
    pub pm: Arc<ProcessManager>,
}

/// 加载配置
#[tauri::command]
async fn load_config() -> Result<LauncherConfig, String> {
    config::load_config()
}

/// 保存配置
#[tauri::command]
async fn save_config(config: LauncherConfig) -> Result<(), String> {
    config::save_config(&config)
}

/// 检测 dashi-taskboard 项目路径是否有效
#[tauri::command]
async fn validate_taskboard_path(path: String) -> Result<bool, String> {
    let p = std::path::Path::new(&path);
    Ok(p.exists()
        && p.join("server/index.mjs").exists()
        && p.join("package.json").exists()
        && p.join("scripts/codex-injector.mjs").exists())
}

/// 检测 Node.js 是否可用并返回版本
#[tauri::command]
async fn check_node_version(node_path: String) -> Result<String, String> {
    let node = if node_path.is_empty() {
        "node".to_string()
    } else {
        node_path
    };
    let output = std::process::Command::new(&node)
        .arg("--version")
        .output()
        .map_err(|e| format!("无法执行 {}: {}", node, e))?;
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    } else {
        Err("Node.js 不可用".to_string())
    }
}

/// 检测 Codex 桌面应用是否存在
#[tauri::command]
async fn check_codex_app(app_path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(&app_path).exists())
}

/// 获取所有进程状态
#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<Vec<ProcessInfo>, String> {
    Ok(state.pm.get_all_status().await)
}

/// 一键启动：先启动 taskboard 服务，再启动 codex 注入器
#[tauri::command]
async fn start_all(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    config: LauncherConfig,
) -> Result<(), String> {
    // 验证路径
    if config.taskboard_path.is_empty() {
        return Err("请先设置 dashi-taskboard 项目路径".to_string());
    }
    if !std::path::Path::new(&config.taskboard_path).exists() {
        return Err(format!("路径不存在: {}", config.taskboard_path));
    }

    // 启动 taskboard 服务
    app.emit("status-update", &serde_json::json!({
        "name": "taskboard-server",
        "status": "starting",
        "message": "正在启动 Taskboard 服务..."
    })).ok();

    state.pm.start_taskboard(
        &config.taskboard_path,
        &config.node_path,
        &config.taskboard_host,
        config.taskboard_port,
    ).await?;

    app.emit("status-update", &serde_json::json!({
        "name": "taskboard-server",
        "status": "running",
        "message": format!("Taskboard 运行在 http://{}:{}", config.taskboard_host, config.taskboard_port)
    })).ok();

    // 等待服务就绪
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // 启动 codex 注入器
    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "starting",
        "message": "正在启动 Codex 注入器..."
    })).ok();

    state.pm.start_injector(
        &config.taskboard_path,
        &config.node_path,
        config.cdp_port,
        &config.codex_app_path,
        config.separate_window_mode,
    ).await?;

    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "running",
        "message": "注入器运行中"
    })).ok();

    Ok(())
}

/// 停止所有服务
#[tauri::command]
async fn stop_all(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "stopping",
        "message": "正在停止注入器..."
    })).ok();

    state.pm.stop_injector().await?;

    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "stopped",
        "message": "已停止"
    })).ok();

    app.emit("status-update", &serde_json::json!({
        "name": "taskboard-server",
        "status": "stopping",
        "message": "正在停止 Taskboard 服务..."
    })).ok();

    state.pm.stop_taskboard().await?;

    app.emit("status-update", &serde_json::json!({
        "name": "taskboard-server",
        "status": "stopped",
        "message": "已停止"
    })).ok();

    Ok(())
}

/// 单独启动 taskboard 服务
#[tauri::command]
async fn start_taskboard(
    state: State<'_, AppState>,
    config: LauncherConfig,
) -> Result<(), String> {
    state.pm.start_taskboard(
        &config.taskboard_path,
        &config.node_path,
        &config.taskboard_host,
        config.taskboard_port,
    ).await
}

/// 单独停止 taskboard 服务
#[tauri::command]
async fn stop_taskboard(state: State<'_, AppState>) -> Result<(), String> {
    state.pm.stop_taskboard().await
}

/// 单独启动 codex 注入器
#[tauri::command]
async fn start_injector(
    state: State<'_, AppState>,
    config: LauncherConfig,
) -> Result<(), String> {
    state.pm.start_injector(
        &config.taskboard_path,
        &config.node_path,
        config.cdp_port,
        &config.codex_app_path,
        config.separate_window_mode,
    ).await
}

/// 单独停止 codex 注入器
#[tauri::command]
async fn stop_injector(state: State<'_, AppState>) -> Result<(), String> {
    state.pm.stop_injector().await
}

/// 在浏览器中打开 Taskboard
#[tauri::command]
async fn open_taskboard(config: LauncherConfig) -> Result<(), String> {
    let url = format!("http://{}:{}", config.taskboard_host, config.taskboard_port);
    #[cfg(unix)]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("无法打开浏览器: {}", e))?;
    }
    Ok(())
}

/// 安装 Codex Skill（创建符号链接）
#[tauri::command]
async fn install_skill(taskboard_path: String) -> Result<String, String> {
    let home = std::env::var("HOME")
        .map_err(|_| "无法获取 HOME".to_string())?;
    let skill_source = format!("{}/skills/manage-taskboard", taskboard_path);
    let skill_target = format!("{}/.codex/skills/manage-taskboard", home);

    // 检查源路径
    if !std::path::Path::new(&skill_source).exists() {
        return Err(format!("Skill 源路径不存在: {}", skill_source));
    }

    // 创建目标目录
    let skills_dir = format!("{}/.codex/skills", home);
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| format!("创建 skills 目录失败: {}", e))?;

    // 如果已存在则先删除
    if std::path::Path::new(&skill_target).exists() {
        #[cfg(unix)]
        {
            std::fs::remove_file(&skill_target)
                .or_else(|_| std::fs::remove_dir_all(&skill_target))
                .map_err(|e| format!("删除旧链接失败: {}", e))?;
        }
    }

    // 创建符号链接
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&skill_source, &skill_target)
            .map_err(|e| format!("创建符号链接失败: {}", e))?;
    }

    Ok(format!("Skill 已安装到 {}", skill_target))
}

/// 运行 taskctl 命令
#[tauri::command]
async fn run_taskctl(
    taskboard_path: String,
    node_path: String,
    args: Vec<String>,
) -> Result<String, String> {
    let node = if node_path.is_empty() {
        "node".to_string()
    } else {
        node_path
    };
    let taskctl_script = format!("{}/cli/taskctl.mjs", taskboard_path);

    let mut cmd = std::process::Command::new(&node);
    cmd.arg(&taskctl_script);
    for arg in &args {
        cmd.arg(arg);
    }
    cmd.current_dir(&taskboard_path);

    let output = cmd.output()
        .map_err(|e| format!("执行 taskctl 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(stderr.is_empty().then(|| stdout).unwrap_or(stderr))
    }
}

/// Updater 配置健康检查
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdaterConfigHealth {
    configured: bool,
    message: String,
}

#[tauri::command]
fn get_updater_config_health(app: tauri::AppHandle) -> UpdaterConfigHealth {
    match app.updater() {
        Ok(_) => UpdaterConfigHealth {
            configured: true,
            message: "updater 配置已就绪".to_string(),
        },
        Err(err) => {
            let message = err.to_string();
            UpdaterConfigHealth {
                configured: false,
                message: if message.is_empty() {
                    "更新源未配置，请在 tauri.conf.json 中设置 updater 的 endpoints 与 pubkey".to_string()
                } else {
                    message
                },
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState {
            pm: Arc::new(ProcessManager::new()),
        })
        .setup(|_app| {
            log::info!("Dashi Taskboard Launcher 启动中...");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // 窗口关闭时尝试停止所有子进程
                let app = window.app_handle();
                let pm = app.state::<AppState>();
                let pm_clone = pm.pm.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = pm_clone.stop_all().await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            validate_taskboard_path,
            check_node_version,
            check_codex_app,
            get_status,
            start_all,
            stop_all,
            start_taskboard,
            stop_taskboard,
            start_injector,
            stop_injector,
            open_taskboard,
            install_skill,
            run_taskctl,
            get_updater_config_health,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Dashi Taskboard Launcher 失败");
}

fn main() {
    run();
}
