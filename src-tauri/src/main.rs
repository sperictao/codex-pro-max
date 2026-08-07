// 发布版按 Windows GUI 子系统链接，否则双击 exe 会附带一个控制台窗口，
// 关掉控制台会把整个进程树（含软件窗体）一起杀掉
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use tauri::{Emitter, Manager, State};
use serde::Serialize;

mod config;
mod codex_guard;
mod fastctx;
mod process_manager;
mod updater;

use config::LauncherConfig;
use process_manager::{ProcessManager, ProcessInfo, resolve_node};

/// 应用共享状态
pub struct AppState {
    pub pm: Arc<ProcessManager>,
}

/// 获取内置 taskboard 路径
/// 打包后：resource_dir/vendor/dashi-taskboard
/// 开发模式：项目根目录/vendor/dashi-taskboard（通过 CARGO_MANIFEST_DIR 回退）
#[tauri::command]
fn get_bundled_taskboard_path(app: tauri::AppHandle) -> Option<String> {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();

    // 1. 打包后的 resource_dir。不同 bundler 版本对 ../vendor 这种项目外
    //    资源的落点不一致（带不带 vendor/ 前缀），两种都探测
    if let Ok(resource_path) = app.path().resource_dir() {
        candidates.push(resource_path.join("vendor").join("dashi-taskboard"));
        candidates.push(resource_path.join("dashi-taskboard"));
    }

    // 2. 开发模式回退：通过 CARGO_MANIFEST_DIR 定位项目根目录
    // src-tauri/Cargo.toml 编译时 CARGO_MANIFEST_DIR = .../src-tauri
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    candidates.push(
        std::path::Path::new(manifest_dir)
            .join("..")
            .join("vendor")
            .join("dashi-taskboard"),
    );

    // ponytail: 用 server/index.mjs 代替裸 exists()，与 validate_taskboard_path 一致
    candidates
        .into_iter()
        .find(|p| p.join("server/index.mjs").exists())
        .map(|p| config::strip_unc(&p.to_string_lossy()))
}

/// 加载配置
#[tauri::command]
async fn load_config() -> Result<LauncherConfig, String> {
    config::load_config()
}

/// 保存配置（全量覆盖，仅前端已知字段的场景使用）
#[tauri::command]
async fn save_config(config: LauncherConfig) -> Result<(), String> {
    config::save_config(&config)
}

/// 仅更新设置类字段，保留 codex_guard 等看守状态不变
/// 防止设置页保存时把内存中过时的看守状态写回，导致 apply/lock 被回滚
#[tauri::command]
async fn update_settings(config: LauncherConfig) -> Result<(), String> {
    let mut current = config::load_config()?;
    config::merge_settings(&mut current, &config);
    config::save_config(&current)
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
    let node = resolve_node(&node_path);
    let mut cmd = std::process::Command::new(&node);
    cmd.arg("--version");
    // Windows 上不弹出终端窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output()
        .map_err(|e| format!("无法执行 {}: {}", node, e))?;
    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version)
    } else {
        Err("Node.js 不可用".to_string())
    }
}

/// exe 名精确匹配（忽略大小写）：OpenAI 直装版/别名固定为 Codex.exe/ChatGPT.exe
/// ponytail: 模糊匹配（含 codex 即收）会把 Codex++ 之类第三方工具认成目标，
/// 抢在商店版 MSIX 回退之前返回，拉起错误的应用（Windows 实机踩坑）
#[cfg(target_os = "windows")]
fn is_codex_exe(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(), "codex.exe" | "chatgpt.exe")
}

/// Codex/ChatGPT 桌面端常见安装位置，按优先级排序
fn codex_app_candidates() -> Vec<String> {
    #[cfg(target_os = "macos")]
    let v = vec![
        "/Applications/ChatGPT.app".to_string(),
        "/Applications/Codex.app".to_string(),
        format!(
            "{}/Applications/ChatGPT.app",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];
    #[cfg(target_os = "windows")]
    let v = {
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let pf = std::env::var("ProgramFiles")
            .unwrap_or_else(|_| "C:\\Program Files".to_string());
        let mut v = vec![
            // Codex 直装版真实安装位置（参考 CodexPlusPlus）
            format!("{}\\OpenAI\\Codex\\bin\\Codex.exe", local),
            format!("{}\\OpenAI\\Codex\\Codex.exe", local),
            format!("{}\\Programs\\OpenAI\\Codex\\Codex.exe", local),
            // 直装版（NSIS 每用户 / 机器级）
            format!("{}\\Programs\\ChatGPT\\ChatGPT.exe", local),
            format!("{}\\Programs\\Codex\\Codex.exe", local),
            format!("{}\\ChatGPT\\ChatGPT.exe", pf),
            format!("{}\\Codex\\Codex.exe", pf),
            // 微软商店版的应用执行别名（reparse point，可直接启动）
            format!("{}\\Microsoft\\WindowsApps\\ChatGPT.exe", local),
            format!("{}\\Microsoft\\WindowsApps\\chatgpt.exe", local),
        ];
        // ponytail: 安装目录随版本有差异，扫 Programs 与 Program Files 下名字含
        // chatgpt/codex 的文件夹预筛；目录名模糊匹配仅作预筛，exe 必须精确名匹配
        // （见 is_codex_exe），注册表卸载键更全但重，不够再升级
        let is_target = |name: &str| name.contains("chatgpt") || name.contains("codex");
        for root in [format!("{}\\Programs", local), pf] {
            if let Ok(entries) = std::fs::read_dir(&root) {
                for dir in entries.flatten() {
                    if !is_target(&dir.file_name().to_string_lossy().to_lowercase()) {
                        continue;
                    }
                    if let Ok(files) = std::fs::read_dir(dir.path()) {
                        for f in files.flatten() {
                            if is_codex_exe(&f.file_name().to_string_lossy()) {
                                v.push(f.path().to_string_lossy().into_owned());
                            }
                        }
                    }
                }
            }
        }
        // 商店版（MSIX）：C:\Program Files\WindowsApps 对普通用户锁死，不可枚举也不可
        // 直接拉起；走每用户别名目录，别名将 --remote-debugging-port 透传给应用
        if let Ok(aliases) = std::fs::read_dir(format!("{}\\Microsoft\\WindowsApps", local)) {
            for f in aliases.flatten() {
                if is_codex_exe(&f.file_name().to_string_lossy()) {
                    v.push(f.path().to_string_lossy().into_owned());
                }
            }
        }
        v
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let v = vec!["/usr/bin/chatgpt".to_string(), "/usr/local/bin/chatgpt".to_string()];
    v
}

/// OpenAI 商店版包族名（2p2nqsd0c76g0 为 OpenAI 签名发布者哈希），
/// 参考 CodexPlusPlus；AMID 即 <包族名>!App，无需解析包全名
#[cfg(target_os = "windows")]
const STORE_PACKAGE_FAMILIES: &[&str] = &[
    "OpenAI.Codex_2p2nqsd0c76g0",
    "OpenAI.CodexBeta_2p2nqsd0c76g0",
    "OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0",
];

/// appmodel API 查包族是否已安装，任意进程可调、无权限要求（不枚举 WindowsApps）
/// ponytail: 发布者哈希变更会漏检，别名目录扫描仍作兜底；出现时再补哈希
#[cfg(target_os = "windows")]
fn package_installed(family_name: &str) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::GetPackagesByPackageFamily;
    let family: Vec<u16> = family_name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut count = 0u32;
    let mut buf_len = 0u32;
    let status = unsafe {
        GetPackagesByPackageFamily(
            PCWSTR(family.as_ptr()),
            &mut count,
            None,
            &mut buf_len,
            None,
        )
    };
    status == ERROR_INSUFFICIENT_BUFFER || (status == ERROR_SUCCESS && count > 0)
}

/// 商店版（MSIX）应用的 AMID 列表，如 OpenAI.Codex_2p2nqsd0c76g0!App
/// 无应用别名时，这是唯一能把 --remote-debugging-port 传到应用命令行的入口
#[cfg(target_os = "windows")]
pub(crate) fn store_app_amids() -> Vec<String> {
    STORE_PACKAGE_FAMILIES
        .iter()
        .filter(|f| package_installed(f))
        .map(|f| format!("{}!App", f))
        .collect()
}

/// 通过 IApplicationActivationManager 激活商店版应用，args 追加到其命令行
/// （CodexPlusPlus 已验证激活参数可达 Electron argv；COM 初始化配对其加固模式）
#[cfg(target_os = "windows")]
pub(crate) fn launch_store_app(amid: &str, args: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        ApplicationActivationManager, IApplicationActivationManager, ACTIVATEOPTIONS,
    };
    unsafe {
        let coinit = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let should_uninit = coinit.is_ok();
        const RPC_E_CHANGED_MODE: i32 = -2147417850;
        coinit
            .ok()
            .or_else(|e| if e.code().0 == RPC_E_CHANGED_MODE { Ok(()) } else { Err(e) })
            .map_err(|e| format!("COM 初始化失败: {}", e))?;
        let result = (|| -> windows::core::Result<()> {
            let mgr: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)?;
            mgr.ActivateApplication(&HSTRING::from(amid), &HSTRING::from(args), ACTIVATEOPTIONS(0))?;
            Ok(())
        })();
        if should_uninit {
            CoUninitialize();
        }
        result.map_err(|e| format!("无法启动商店版应用 ({}): {}", amid, e))
    }
}

/// 自动探测 Codex 桌面应用，返回第一个真实存在的路径
#[tauri::command]
fn detect_codex_app() -> Option<String> {
    let found = codex_app_candidates()
        .into_iter()
        .find(|p| std::path::Path::new(p).exists());
    // 商店版无文件路径，返回 msix: 哨兵，ensure_codex_cdp 按此前缀走 COM 激活
    #[cfg(target_os = "windows")]
    let found = found.or_else(|| store_app_amids().into_iter().next().map(|a| format!("msix:{}", a)));
    found
}

/// 检测 Codex 桌面应用是否存在
/// 支持检查指定路径 + 搜索常见安装位置
#[tauri::command]
async fn check_codex_app(app_path: String) -> Result<bool, String> {
    // 1. 检查用户指定的路径
    if !app_path.is_empty() && std::path::Path::new(&app_path).exists() {
        return Ok(true);
    }

    // 商店版哨兵：AMID 仍装在系统上即有效
    #[cfg(target_os = "windows")]
    if let Some(amid) = app_path.strip_prefix("msix:") {
        return Ok(store_app_amids().iter().any(|a| a == amid));
    }

    // 2. 搜索常见安装位置
    for candidate in codex_app_candidates() {
        if std::path::Path::new(&candidate).exists() {
            return Ok(true);
        }
    }

    Ok(false)
}

/// 获取所有进程状态
#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<Vec<ProcessInfo>, String> {
    Ok(state.pm.get_all_status().await)
}

/// 一键启动的共享实现：Tauri 命令与托盘菜单共用
async fn run_start_all(
    pm: &ProcessManager,
    app: &tauri::AppHandle,
    config: &LauncherConfig,
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

    pm.start_taskboard(
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

    pm.start_injector(
        &config.taskboard_path,
        &config.node_path,
        config.cdp_port,
        &config.codex_app_path,
        config.separate_window_mode,
        config.taskboard_port,
    ).await?;

    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "running",
        "message": "注入器运行中"
    })).ok();

    Ok(())
}

/// 全部停止的共享实现：Tauri 命令与托盘菜单共用
async fn run_stop_all(pm: &ProcessManager, app: &tauri::AppHandle) -> Result<(), String> {
    app.emit("status-update", &serde_json::json!({
        "name": "codex-injector",
        "status": "stopping",
        "message": "正在停止注入器..."
    })).ok();

    pm.stop_injector().await?;

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

    pm.stop_taskboard().await?;

    app.emit("status-update", &serde_json::json!({
        "name": "taskboard-server",
        "status": "stopped",
        "message": "已停止"
    })).ok();

    Ok(())
}

/// 一键启动：先启动 taskboard 服务，再启动 codex 注入器
#[tauri::command]
async fn start_all(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    config: LauncherConfig,
) -> Result<(), String> {
    run_start_all(&state.pm, &app, &config).await
}

/// 停止所有服务
#[tauri::command]
async fn stop_all(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    run_stop_all(&state.pm, &app).await
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
        config.taskboard_port,
    ).await
}

/// 单独停止 codex 注入器
#[tauri::command]
async fn stop_injector(state: State<'_, AppState>) -> Result<(), String> {
    state.pm.stop_injector().await
}

/// 跨平台获取用户主目录
fn home_dir() -> Result<String, String> {
    #[cfg(unix)]
    {
        std::env::var("HOME").map_err(|_| "无法获取 HOME 环境变量".to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").map_err(|_| "无法获取 USERPROFILE 环境变量".to_string())
    }
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
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW：GUI 应用拉起 cmd 不能闪控制台窗口
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "start", "", &url]).creation_flags(CREATE_NO_WINDOW);
        cmd.spawn().map_err(|e| format!("无法打开浏览器: {}", e))?;
    }
    Ok(())
}

/// Skill 安装状态
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SkillStatus {
    /// installed | not-installed | mismatch
    state: String,
    detail: String,
    target_path: String,
}

/// 检测 manage-taskboard Skill 的安装状态
#[tauri::command]
async fn check_skill_status(taskboard_path: String) -> Result<SkillStatus, String> {
    let home = home_dir()?;
    let target = std::path::Path::new(&home).join(".codex/skills/manage-taskboard");
    let source = std::path::Path::new(&taskboard_path).join("skills/manage-taskboard");
    let target_path = target.display().to_string();

    let meta = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(_) => {
            return Ok(SkillStatus {
                state: "not-installed".to_string(),
                detail: "未安装".to_string(),
                target_path,
            })
        }
    };

    if meta.file_type().is_symlink() {
        let link = std::fs::read_link(&target)
            .map_err(|e| format!("读取符号链接失败: {}", e))?;
        // read_link 可能返回相对路径，统一与 source 比较前先做字典序归一
        let link_norm = link.canonicalize().unwrap_or(link);
        let source_norm = source.canonicalize().unwrap_or(source);
        if link_norm == source_norm {
            Ok(SkillStatus {
                state: "installed".to_string(),
                detail: "已安装，指向当前 Taskboard 仓库".to_string(),
                target_path,
            })
        } else {
            Ok(SkillStatus {
                state: "mismatch".to_string(),
                detail: format!("符号链接指向 {}，与当前 Taskboard 路径不一致", link_norm.display()),
                target_path,
            })
        }
    } else if target.join("SKILL.md").exists() {
        Ok(SkillStatus {
            state: "installed".to_string(),
            detail: "已安装（实体目录）".to_string(),
            target_path,
        })
    } else {
        Ok(SkillStatus {
            state: "mismatch".to_string(),
            detail: "目标路径已存在但不是有效的 Skill".to_string(),
            target_path,
        })
    }
}

/// 安装 Codex Skill（创建符号链接）
#[tauri::command]
async fn install_skill(taskboard_path: String) -> Result<String, String> {
    let home = home_dir()?;
    let skill_source = std::path::Path::new(&taskboard_path).join("skills/manage-taskboard");
    let skill_target = std::path::Path::new(&home).join(".codex/skills/manage-taskboard");

    // 检查源路径
    if !skill_source.exists() {
        return Err(format!("Skill 源路径不存在: {}", skill_source.display()));
    }

    // 创建目标目录
    let skills_dir = std::path::Path::new(&home).join(".codex/skills");
    std::fs::create_dir_all(&skills_dir)
        .map_err(|e| format!("创建 skills 目录失败: {}", e))?;

    // 如果已存在则先删除
    if skill_target.exists() {
        std::fs::remove_file(&skill_target)
            .or_else(|_| std::fs::remove_dir_all(&skill_target))
            .map_err(|e| format!("删除旧链接失败: {}", e))?;
    }

    // 创建符号链接（跨平台）
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&skill_source, &skill_target)
            .map_err(|e| format!("创建符号链接失败: {}", e))?;
    }
    #[cfg(windows)]
    {
        // Windows 上根据源类型选择 symlink_file 或 symlink_dir
        let result = if skill_source.is_dir() {
            std::os::windows::fs::symlink_dir(&skill_source, &skill_target)
        } else {
            std::os::windows::fs::symlink_file(&skill_source, &skill_target)
        };
        result.map_err(|e| format!("创建符号链接失败: {}（可能需要管理员权限或开启开发者模式）", e))?;
    }

    Ok(format!("Skill 已安装到 {}", skill_target.display()))
}

/// 运行 taskctl 命令
#[tauri::command]
async fn run_taskctl(
    taskboard_path: String,
    node_path: String,
    args: Vec<String>,
) -> Result<String, String> {
    let node = resolve_node(&node_path);
    let taskctl_script = format!("{}/cli/taskctl.mjs", taskboard_path);

    let mut cmd = std::process::Command::new(&node);
    cmd.arg(&taskctl_script);
    for arg in &args {
        cmd.arg(arg);
    }
    cmd.current_dir(&taskboard_path);
    // Windows 上不弹出终端窗口
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd.output()
        .map_err(|e| format!("执行 taskctl 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// 显示并聚焦主窗口（托盘点击 / 菜单）
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        let _ = app.show();
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 隐藏主窗口到托盘
fn hide_main_window_to_tray(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    // macOS 上连 Dock 图标一起隐去，驻留托盘才完整
    #[cfg(target_os = "macos")]
    let _ = app.hide();
}

/// 创建系统托盘（图标 + 菜单 + 事件）
fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    let show = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
    let start = MenuItemBuilder::with_id("start-all", "一键启动").build(app)?;
    let stop = MenuItemBuilder::with_id("stop-all", "全部停止").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&start)
        .item(&stop)
        .separator()
        .item(&quit)
        .build()?;

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Dashi Taskboard Launcher")
        .show_menu_on_left_click(false);
    // ponytail: 直接用应用图标；macOS 菜单栏想更精致可换 template 图标
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id().as_ref() {
        "show" => show_main_window(app),
        "start-all" => {
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let pm = app_handle.state::<AppState>().pm.clone();
                match config::load_config() {
                    Ok(cfg) => {
                        if let Err(e) = run_start_all(&pm, &app_handle, &cfg).await {
                            log::error!("托盘一键启动失败: {}", e);
                        }
                    }
                    Err(e) => log::error!("托盘一键启动读取配置失败: {}", e),
                }
            });
        }
        "stop-all" => {
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let pm = app_handle.state::<AppState>().pm.clone();
                if let Err(e) = run_stop_all(&pm, &app_handle).await {
                    log::error!("托盘全部停止失败: {}", e);
                }
            });
        }
        "quit" => {
            let app_handle = app.clone();
            tauri::async_runtime::spawn(async move {
                let pm = app_handle.state::<AppState>().pm.clone();
                let _ = pm.stop_all().await;
                app_handle.exit(0);
            });
        }
        _ => {}
    })
    .on_tray_icon_event(|tray, event| {
        if let tauri::tray::TrayIconEvent::Click {
            button: tauri::tray::MouseButton::Left,
            button_state: tauri::tray::MouseButtonState::Up,
            ..
        } = event
        {
            show_main_window(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
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
        .manage(updater::PendingUpdateState::default())
        .setup(|app| {
            log::info!("Dashi Taskboard Launcher 启动中...");
            if let Err(e) = setup_tray(app) {
                log::error!("初始化系统托盘失败: {}", e);
            }
            tauri::async_runtime::spawn(codex_guard::poll_loop());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = config::load_config()
                    .map(|c| c.minimize_to_tray_on_close)
                    .unwrap_or(false);
                if minimize_to_tray {
                    // 阻止关闭，窗口隐入托盘，子进程继续运行
                    api.prevent_close();
                    hide_main_window_to_tray(window.app_handle());
                } else {
                    // 窗口关闭时尝试停止所有子进程
                    let app = window.app_handle();
                    let pm = app.state::<AppState>();
                    let pm_clone = pm.pm.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = pm_clone.stop_all().await;
                    });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_bundled_taskboard_path,
            load_config,
            save_config,
            update_settings,
            validate_taskboard_path,
            check_node_version,
            detect_codex_app,
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
            check_skill_status,
            run_taskctl,
            codex_guard::guard_get_view,
            codex_guard::guard_set_enabled,
            codex_guard::guard_set_value,
            codex_guard::guard_apply,
            codex_guard::guard_set_locked,
            codex_guard::guard_add_custom_param,
            codex_guard::guard_remove_custom_param,
            codex_guard::guard_get_schema_file_path,
            codex_guard::guard_get_files,
            codex_guard::guard_add_file,
            codex_guard::guard_update_file,
            codex_guard::guard_remove_file,
            codex_guard::guard_detect_file,
            codex_guard::guard_relativize_picked_path,
            fastctx::fastctx_detect,
            fastctx::fastctx_apply,
            fastctx::fastctx_unapply,
            fastctx::fastctx_open_console,
            updater::get_updater_config_health,
            updater::get_updater_help_paths,
            updater::check_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("启动 Dashi Taskboard Launcher 失败");
}

fn main() {
    run();
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::is_codex_exe;

    #[test]
    fn codex_exe_requires_exact_name() {
        // OpenAI 直装版/商店别名的固定命名（大小写不敏感）
        assert!(is_codex_exe("Codex.exe"));
        assert!(is_codex_exe("chatgpt.exe"));
        assert!(is_codex_exe("CHATGPT.EXE"));
        // 名字带 codex 的第三方工具不得命中（Codex++ 实机误检回归）
        assert!(!is_codex_exe("codex-plus-plus.exe"));
        assert!(!is_codex_exe("codex-plus-plus-manager.exe"));
        // 其它形似项
        assert!(!is_codex_exe("uninstall.exe"));
        assert!(!is_codex_exe("codex.exe.bak"));
        assert!(!is_codex_exe("codex"));
    }
}
