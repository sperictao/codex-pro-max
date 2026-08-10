//! 轮询（60s，仅 launcher 运行期间）：锁定参数漂移即自动恢复（写入前备份），
//! 恢复记录落盘，不弹通知（CONTEXT.md 语义边界）

use std::collections::BTreeMap;

use crate::config::ConfigStore;

use super::atomic_store::PlatformAtomicFileWriter;
use super::engine::{
    check_many, execute_transaction_batch, expected_of, prepare_file_plan, ManagedMember,
    TransactionWrite,
};
use super::files::load_files;
use super::journal::JournalParticipant;
use super::now_secs;
use super::ownership::validate_ownership;
use super::schema::load_schema;
use super::{AppPaths, GuardCoordinator, GuardParam};

fn handle_restore_error(
    coordinator: &GuardCoordinator,
    param_id: &str,
    error: String,
) -> Result<(), String> {
    log::error!("codex guard 恢复 {} 失败: {}", param_id, error);
    if error.starts_with("guard transaction failed: ") {
        coordinator.mark_recovery_blocked("recovery_failed");
    }
    Err(error)
}

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
    let Some(_write) = coordinator.try_poll_write()? else {
        return Ok(());
    };
    let schema = load_schema(store)?;
    // 只看守文件列表内的目标文件，与 UI 可见范围一致（CONTEXT.md：UI 完全由合并结果驱动）
    let files = load_files(store)?;
    validate_ownership(paths, &files, &schema).map_err(|error| error.to_string())?;
    let result = store.with_launcher_transaction(|launcher| {
        if !launcher.config().codex_guard.enabled
            || !launcher
                .config()
                .codex_guard
                .params
                .values()
                .any(|state| state.locked)
        {
            return Ok(());
        }

        // BTreeMap 固定物理文件的计划顺序；成员顺序再由 plan_file_write 固定，
        // 这样 schema/files 的输入顺序变化不会改变 journal 或候选内容。
        let mut file_members = BTreeMap::<
            String,
            (super::GuardFileFormat, Vec<GuardParam>),
        >::new();
        for file in &files {
            let locked_params = schema
                .iter()
                .filter(|param| {
                    param.file == file.file
                        && launcher
                            .config()
                            .codex_guard
                            .params
                            .get(&param.id)
                            .is_some_and(|state| state.locked)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !locked_params.is_empty() {
                file_members.insert(file.file.clone(), (file.format, locked_params));
            }
        }

        let checked_at = now_secs();
        let mut writes = Vec::new();
        let mut restored_ids = Vec::new();
        let codex_writer = PlatformAtomicFileWriter;

        for (relative_file, (format, locked_params)) in file_members {
            let expected_values = locked_params
                .iter()
                .map(|param| {
                    expected_of(param, launcher.config().codex_guard.params.get(&param.id))
                })
                .collect::<Vec<_>>();
            let check_targets = locked_params
                .iter()
                .zip(expected_values.iter())
                .collect::<Vec<_>>();
            let check_results = check_many(paths, &relative_file, format, &check_targets);
            let mut drift_members = Vec::new();
            let mut file_has_error = false;

            for ((param, expected), check_result) in locked_params
                .iter()
                .zip(expected_values.iter())
                .zip(check_results)
            {
                let state = launcher
                    .config_mut()
                    .codex_guard
                    .params
                    .entry(param.id.clone())
                    .or_default();
                state.last_checked = Some(checked_at);
                match check_result.status.as_str() {
                    "drift" | "missing" => drift_members.push(ManagedMember {
                        id: param.id.clone(),
                        apply_mode: param.apply_mode.clone(),
                        path: param.path.clone(),
                        value_type: param.value_type.clone(),
                        expected: expected.clone(),
                    }),
                    "error" => {
                        file_has_error = true;
                        log::error!(
                            "codex guard 轮询校验 {} 失败: {}",
                            param.id,
                            check_result.error.unwrap_or_else(|| "unknown error".into())
                        );
                    }
                    _ => {}
                }
            }

            if file_has_error || drift_members.is_empty() {
                continue;
            }

            let (target, original, plan) =
                prepare_file_plan(paths, &relative_file, format, &drift_members)?;
            if !plan.changed {
                continue;
            }
            writes.push(TransactionWrite {
                participant: JournalParticipant::Codex,
                relative_file: plan.relative_file,
                target,
                original,
                candidate: plan.candidate,
                writer: &codex_writer,
            });
            restored_ids.extend(drift_members.into_iter().map(|member| member.id));
        }

        // last_checked/last_restored 与 Codex 候选一起提交；如果联合事务失败，
        // with_launcher_transaction 不会把这些内存状态写回 config.json。
        let restored_at = now_secs();
        for id in &restored_ids {
            let state = launcher
                .config_mut()
                .codex_guard
                .params
                .entry(id.clone())
                .or_default();
            state.last_restored = Some(restored_at);
        }
        writes.push(TransactionWrite {
            participant: JournalParticipant::Launcher,
            relative_file: "config.json".into(),
            target: launcher.target().to_path_buf(),
            original: launcher.original().map(ToOwned::to_owned),
            candidate: launcher.candidate_bytes()?,
            writer: launcher.writer(),
        });

        execute_transaction_batch(paths, writes)
    });

    match result {
        Ok(()) => Ok(()),
        Err(error) => handle_restore_error(coordinator, "poll", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::ownership::normalize_toml_path;

    fn configure_locked_params(store: &ConfigStore, ids: &[&str]) {
        store
            .update_launcher(|config| {
                config.codex_guard.enabled = true;
                for id in ids {
                    let state = config
                        .codex_guard
                        .params
                        .entry((*id).to_string())
                        .or_default();
                    state.applied = true;
                    state.locked = true;
                }
                Ok(())
            })
            .unwrap();
    }

    fn toml_value(path: &std::path::Path, key: &str) -> toml_edit::Item {
        let document = std::fs::read_to_string(path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        let normalized = normalize_toml_path(key).unwrap();
        super::super::toml_ops::get_toml_path(&document, &normalized)
            .unwrap()
            .clone()
    }

    #[test]
    fn poll_merges_multiple_drifted_members_in_one_toml_candidate() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(
            &target,
            "[features.multi_agent_v2]\nenabled = false\nhide_spawn_agent_metadata = false\n",
        )
        .unwrap();
        let store = ConfigStore::new(paths.clone());
        let ids = [
            "features.multi_agent_v2.enabled",
            "features.multi_agent_v2.hide_spawn_agent_metadata",
        ];
        configure_locked_params(&store, &ids);

        poll_once(&store, &paths, &GuardCoordinator::new()).unwrap();

        assert_eq!(
            toml_value(&target, "features.multi_agent_v2.enabled").as_bool(),
            Some(true)
        );
        assert_eq!(
            toml_value(&target, "features.multi_agent_v2.hide_spawn_agent_metadata").as_bool(),
            Some(true)
        );
        let config = store.load_launcher().unwrap();
        for id in ids {
            let state = config.codex_guard.params.get(id).unwrap();
            assert!(state.last_checked.is_some());
            assert!(state.last_restored.is_some());
        }
    }

    #[test]
    fn poll_does_not_write_codex_when_all_locked_members_match() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let target = paths.codex_file("config.toml");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = b"[features.multi_agent_v2]\nenabled = true\n";
        std::fs::write(&target, original).unwrap();
        let store = ConfigStore::new(paths.clone());
        configure_locked_params(&store, &["features.multi_agent_v2.enabled"]);

        poll_once(&store, &paths, &GuardCoordinator::new()).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), original);
        assert!(store
            .load_launcher()
            .unwrap()
            .codex_guard
            .params
            .get("features.multi_agent_v2.enabled")
            .unwrap()
            .last_checked
            .is_some());
    }

    #[test]
    fn transaction_restore_failure_blocks_follow_up_writes() {
        let coordinator = GuardCoordinator::new();
        let error = handle_restore_error(
            &coordinator,
            "subagent.model",
            "guard transaction failed: replace_failed".to_string(),
        )
        .unwrap_err();

        assert_eq!(error, "guard transaction failed: replace_failed");
        assert_eq!(
            coordinator.recovery_status(),
            super::super::GuardRecoveryStatus {
                blocked: true,
                code: Some("recovery_failed".to_string()),
            }
        );
        assert_eq!(
            coordinator.try_guard_write().unwrap_err(),
            "recovery_blocked"
        );
    }
}
