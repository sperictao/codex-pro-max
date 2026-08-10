//! 轮询（60s，仅 launcher 运行期间）：锁定参数漂移即自动恢复（写入前备份），
//! 恢复记录落盘，不弹通知（CONTEXT.md 语义边界）

use crate::config::ConfigStore;

use super::engine::{check_many, execute_single_plan, expected_of};
use super::files::load_files;
use super::now_secs;
use super::ownership::validate_ownership;
use super::schema::load_schema;
use super::{AppPaths, GuardCoordinator};

pub async fn poll_loop(store: ConfigStore, paths: AppPaths, coordinator: GuardCoordinator) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Err(e) = poll_once(&store, &paths, &coordinator) {
            log::error!("codex guard 轮询失败: {}", e);
        }
    }
}

fn poll_once(
    store: &ConfigStore,
    paths: &AppPaths,
    coordinator: &GuardCoordinator,
) -> Result<(), String> {
    let _write = match coordinator.try_write() {
        Ok(guard) => guard,
        Err(error) if error == "guard_busy" => return Ok(()),
        Err(error) => return Err(error),
    };
    let snapshot = store.load_launcher()?;
    if !snapshot.codex_guard.enabled
        || !snapshot
            .codex_guard
            .params
            .values()
            .any(|state| state.locked)
    {
        return Ok(());
    }
    let schema = load_schema(store)?;
    // 只看守文件列表内的目标文件，与 UI 可见范围一致（CONTEXT.md：UI 完全由合并结果驱动）
    let files = load_files(store)?;
    validate_ownership(paths, &files, &schema).map_err(|error| error.to_string())?;
    store.update_launcher(|config| {
        if !config.codex_guard.enabled {
            return Ok(());
        }
        for file in &files {
            let locked_params = schema
                .iter()
                .filter(|param| {
                    param.file == file.file
                        && config
                            .codex_guard
                            .params
                            .get(&param.id)
                            .is_some_and(|state| state.locked)
                })
                .collect::<Vec<_>>();
            if locked_params.is_empty() {
                continue;
            }
            let expected_values = locked_params
                .iter()
                .map(|param| expected_of(param, config.codex_guard.params.get(&param.id)))
                .collect::<Vec<_>>();
            let check_targets = locked_params
                .iter()
                .zip(expected_values.iter())
                .map(|(param, expected)| (*param, expected))
                .collect::<Vec<_>>();
            let check_results = check_many(paths, &file.file, file.format, &check_targets);

            for ((param, expected), check_result) in locked_params
                .into_iter()
                .zip(expected_values)
                .zip(check_results)
            {
                let state = config
                    .codex_guard
                    .params
                    .entry(param.id.clone())
                    .or_default();
                state.last_checked = Some(now_secs());
                if check_result.status == "drift" || check_result.status == "missing" {
                    match execute_single_plan(paths, param, file.format, &expected) {
                        Ok(()) => {
                            state.last_restored = Some(now_secs());
                            log::info!("codex guard 已自动恢复: {}", param.id);
                        }
                        Err(error) => log::error!("codex guard 恢复 {} 失败: {}", param.id, error),
                    }
                }
            }
        }
        Ok(())
    })
}
