//! 轮询（60s，仅 launcher 运行期间）：锁定参数漂移即自动恢复（写入前备份），
//! 恢复记录落盘，不弹通知（CONTEXT.md 语义边界）

use crate::config::ConfigStore;

use super::engine::{apply, check, expected_of};
use super::files::load_files;
use super::now_secs;
use super::schema::load_schema;
use super::AppPaths;

pub async fn poll_loop(store: ConfigStore, paths: AppPaths) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Err(e) = poll_once(&store, &paths) {
            log::error!("codex guard 轮询失败: {}", e);
        }
    }
}

fn poll_once(store: &ConfigStore, paths: &AppPaths) -> Result<(), String> {
    let snapshot = store.load_launcher()?;
    if !snapshot.codex_guard.enabled
        || !snapshot.codex_guard.params.values().any(|state| state.locked)
    {
        return Ok(());
    }
    let schema = load_schema(store)?;
    // 只看守文件列表内的目标文件，与 UI 可见范围一致（CONTEXT.md：UI 完全由合并结果驱动）
    let files = load_files(store)?;
    store.update_launcher(|config| {
        if !config.codex_guard.enabled {
            return Ok(());
        }
        for param in &schema {
            if !files.iter().any(|file| file.file == param.file) {
                continue;
            }
            let locked = config
                .codex_guard
                .params
                .get(&param.id)
                .is_some_and(|state| state.locked);
            if !locked {
                continue;
            }
            let expected = expected_of(param, config.codex_guard.params.get(&param.id));
            let check_result = check(paths, param, &expected);
            let state = config.codex_guard.params.entry(param.id.clone()).or_default();
            state.last_checked = Some(now_secs());
            if check_result.status == "drift" || check_result.status == "missing" {
                match apply(paths, param, &expected) {
                    Ok(()) => {
                        state.last_restored = Some(now_secs());
                        log::info!("codex guard 已自动恢复: {}", param.id);
                    }
                    Err(error) => log::error!("codex guard 恢复 {} 失败: {}", param.id, error),
                }
            }
        }
        Ok(())
    })
}
