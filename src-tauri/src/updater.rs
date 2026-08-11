//! 应用更新（参考 one-publish 实现）
//! 检查更新 / 下载重试 / 进度事件 / 安装，更新对象缓存在后端避免重复请求。

use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Error as UpdaterError, Update, UpdaterExt};
use tokio::time::sleep;

use crate::i18n::{tr, trf};

const UPDATE_CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 15);
const UPDATE_DOWNLOAD_MAX_ATTEMPTS: usize = 3;
const PROGRESS_EVENT: &str = "updater-download-progress";

#[derive(Default)]
pub struct PendingUpdateState {
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub available_version: Option<String>,
    pub has_update: bool,
    pub release_notes: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterConfigHealth {
    pub configured: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterHelpPaths {
    pub docs_path: String,
    pub template_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    stage: String, // downloading | retrying | installing
    version: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    percent: Option<f64>,
    attempt: usize,
    max_attempts: usize,
}

fn lock_pending(state: &PendingUpdateState) -> MutexGuard<'_, Option<Update>> {
    state.pending.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn map_updater_error(err: UpdaterError) -> String {
    match err {
        UpdaterError::EmptyEndpoints => {
            tr("Update source not configured; set updater endpoints and pubkey in tauri.conf.json")
        }
        UpdaterError::InsecureTransportProtocol => tr("Update URL must use https"),
        _ => err.to_string(),
    }
}

fn emit_progress(
    app: &AppHandle,
    stage: &str,
    version: &str,
    downloaded: u64,
    total: Option<u64>,
    attempt: usize,
) {
    let percent = total
        .filter(|t| *t > 0)
        .map(|t| ((downloaded as f64 / t as f64) * 100.0).min(100.0));
    let _ = app.emit(
        PROGRESS_EVENT,
        DownloadProgress {
            stage: stage.to_string(),
            version: version.to_string(),
            downloaded_bytes: downloaded,
            total_bytes: total,
            percent,
            attempt,
            max_attempts: UPDATE_DOWNLOAD_MAX_ATTEMPTS,
        },
    );
}

fn retry_delay(attempt: usize) -> Duration {
    match attempt {
        1 => Duration::from_secs(1),
        2 => Duration::from_secs(2),
        _ => Duration::from_secs(4),
    }
}

/// 网络类错误可重试；其余（签名校验失败等）直接放弃
fn is_retryable(err: &UpdaterError) -> bool {
    match err {
        UpdaterError::Reqwest(e) => {
            e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_decode()
        }
        UpdaterError::Network(_) => true,
        _ => false,
    }
}

async fn fetch_remote_update(
    app: &AppHandle,
    state: &PendingUpdateState,
) -> Result<Option<Update>, String> {
    let updater = app
        .updater_builder()
        .timeout(UPDATE_CHECK_TIMEOUT)
        .build()
        .map_err(|e| {
            trf(
                "Update source not configured or unavailable: {error}",
                &[("error", map_updater_error(e))],
            )
        })?;
    let maybe = updater
        .check()
        .await
        .map_err(|e| {
            trf(
                "Failed to check for updates: {error}",
                &[("error", map_updater_error(e))],
            )
        })?
        .map(|mut u| {
            u.timeout = Some(UPDATE_DOWNLOAD_TIMEOUT);
            u
        });
    *lock_pending(state) = maybe.clone();
    Ok(maybe)
}

async fn download_with_retry(app: &AppHandle, update: &Update) -> Result<Vec<u8>, String> {
    let mut last_err: Option<String> = None;
    let mut attempts_used = 1usize;
    for attempt in 1..=UPDATE_DOWNLOAD_MAX_ATTEMPTS {
        attempts_used = attempt;
        emit_progress(app, "downloading", &update.version, 0, None, attempt);
        let app_handle = app.clone();
        let version = update.version.clone();
        let mut downloaded: u64 = 0;
        let result = update
            .download(
                move |chunk_len, total| {
                    downloaded += chunk_len as u64;
                    emit_progress(
                        &app_handle,
                        "downloading",
                        &version,
                        downloaded,
                        total,
                        attempt,
                    );
                },
                || {},
            )
            .await;
        match result {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                log::warn!(
                    "下载更新失败（第 {}/{} 次）: {}",
                    attempt,
                    UPDATE_DOWNLOAD_MAX_ATTEMPTS,
                    e
                );
                if attempt < UPDATE_DOWNLOAD_MAX_ATTEMPTS && is_retryable(&e) {
                    emit_progress(app, "retrying", &update.version, 0, None, attempt + 1);
                    sleep(retry_delay(attempt)).await;
                    continue;
                }
                last_err = Some(map_updater_error(e));
                break;
            }
        }
    }
    let note = if attempts_used > 1 {
        tr(" (retried automatically)")
    } else {
        String::new()
    };
    Err(trf(
        "Download failed{note}: {error}",
        &[("note", note), ("error", last_err.unwrap_or_default())],
    ))
}

/// 两份更新元数据是否变化（版本 / 下载地址 / 签名任一不同即视为新包，值得重试）
fn metadata_changed(prev: &Update, next: &Update) -> bool {
    next.version != prev.version
        || next.download_url != prev.download_url
        || next.signature != prev.signature
}

/// 定位 updater 配置指南（开发仓库内可用，向上逐级查找）
fn resolve_help_paths() -> Result<(PathBuf, PathBuf), String> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    for root in roots {
        let mut current = root.as_path();
        loop {
            let docs = current.join("docs").join("updater").join("SETUP.md");
            let template = current
                .join("src-tauri")
                .join("tauri.conf.updater.example.json");
            if docs.exists() && template.exists() {
                return Ok((docs, template));
            }
            match current.parent() {
                Some(p) => current = p,
                None => break,
            }
        }
    }
    Err(tr(
        "Updater guide files not found; please run this feature from the source repository",
    ))
}

#[tauri::command]
pub fn get_updater_help_paths() -> Result<UpdaterHelpPaths, String> {
    let (docs, template) = resolve_help_paths()?;
    Ok(UpdaterHelpPaths {
        docs_path: docs.to_string_lossy().to_string(),
        template_path: template.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn get_updater_config_health(app: AppHandle) -> UpdaterConfigHealth {
    match app.updater() {
        Ok(_) => UpdaterConfigHealth {
            configured: true,
            message: tr("Updater configuration is ready"),
        },
        Err(e) => UpdaterConfigHealth {
            configured: false,
            message: trf(
                "Update source not configured or unavailable: {error}",
                &[("error", map_updater_error(e))],
            ),
        },
    }
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, PendingUpdateState>,
) -> Result<UpdateInfo, String> {
    match fetch_remote_update(&app, state.inner()).await {
        Ok(Some(u)) => Ok(UpdateInfo {
            current_version: u.current_version.clone(),
            available_version: Some(u.version.clone()),
            has_update: true,
            release_notes: u.body.clone(),
            message: Some(tr("Update available")),
        }),
        Ok(None) => Ok(UpdateInfo {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            available_version: None,
            has_update: false,
            release_notes: None,
            message: None,
        }),
        Err(e) => {
            *lock_pending(state.inner()) = None;
            Ok(UpdateInfo {
                current_version: env!("CARGO_PKG_VERSION").to_string(),
                available_version: None,
                has_update: false,
                release_notes: None,
                message: Some(e),
            })
        }
    }
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, PendingUpdateState>,
    expected_version: Option<String>,
) -> Result<String, String> {
    // 优先用 check 阶段缓存的更新；版本对不上则重新拉取
    let cached = lock_pending(state.inner()).clone();
    let expected = expected_version
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let (update, used_cached) = match cached.filter(|u| {
        expected
            .as_deref()
            .map(|v| v == u.version.as_str())
            .unwrap_or(true)
    }) {
        Some(u) => (u, true),
        None => match fetch_remote_update(&app, state.inner()).await? {
            Some(u) => (u, false),
            None => return Ok(tr("Already up to date; nothing to install")),
        },
    };

    // 缓存的包可能已损坏：下载失败且用的是缓存时，重新拉取元数据，
    // 若远端包有变化则换新包再下载一轮
    let (update, bytes) = match download_with_retry(&app, &update).await {
        Ok(bytes) => (update, bytes),
        Err(first_err) => {
            let refreshed = if used_cached {
                *lock_pending(state.inner()) = None;
                fetch_remote_update(&app, state.inner())
                    .await
                    .ok()
                    .flatten()
                    .filter(|u| metadata_changed(&update, u))
            } else {
                None
            };
            match refreshed {
                Some(u) => {
                    let bytes = download_with_retry(&app, &u).await?;
                    (u, bytes)
                }
                None => return Err(first_err),
            }
        }
    };

    let total = bytes.len() as u64;
    emit_progress(&app, "installing", &update.version, total, Some(total), 1);
    let version = update.version.clone();
    update.install(bytes).map_err(|e| {
        trf(
            "Failed to install update: {error}",
            &[("error", map_updater_error(e))],
        )
    })?;
    *lock_pending(state.inner()) = None;

    // 安装完成，停掉子进程并自动重启
    emit_progress(&app, "restarting", &version, total, Some(total), 1);
    let pm = app.state::<crate::AppState>().pm.clone();
    let _ = pm.stop_all().await;
    log::info!("更新安装完成（v{}），正在重启应用", version);
    app.restart();
}
