//! Tauri 命令：前端调用的全部入口（参数管理 / 自定义参数 / 看守文件管理 / 路径检测）

use crate::i18n::{tr, trf};
use crate::AppState;
use tauri::State;

use super::engine::{
    check, execute_single_plan, expected_of, recover_pending_transactions,
};
use super::files::{detect_file_path, find_file, load_files, update_files};
use super::ownership::{normalize_relative_path, validate_ownership, validate_target_path};
use super::schema::{ensure_schema_file, load_schema, schema_file_path, update_disk_schema};
use super::validate::{
    normalize_custom_id, validate_guard_file, validate_param_fields, validate_param_for_file,
};
use super::view::{build_view, GuardView};
use super::{
    now_secs, DetectRecord, GuardFile, GuardFileFormat, GuardParam, GuardRecoveryStatus,
};

fn find_param(schema: &[GuardParam], id: &str) -> Result<GuardParam, String> {
    schema.iter().find(|p| p.id == id).cloned().ok_or_else(|| {
        trf(
            "Parameter not found in schema: {id}",
            &[("id", id.to_string())],
        )
    })
}

fn find_format(files: &[GuardFile], relative_file: &str) -> Result<GuardFileFormat, String> {
    files
        .iter()
        .find(|file| file.file == relative_file)
        .map(|file| file.format)
        .ok_or_else(|| {
            trf(
                "Target file not found in guard list: {file}",
                &[("file", relative_file.to_string())],
            )
        })
}

fn validate_configuration(
    paths: &super::AppPaths,
    files: &[GuardFile],
    schema: &[GuardParam],
) -> Result<(), String> {
    validate_ownership(paths, files, schema).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn guard_get_view(state: State<'_, AppState>) -> Result<GuardView, String> {
    build_view(&state.config_store, &state.paths)
}

/// 返回启动恢复是否阻断了 Guard 写入。只暴露稳定 code，不泄漏 journal 细节。
#[tauri::command]
#[specta::specta]
pub fn guard_get_recovery_status(
    state: State<'_, AppState>,
) -> Result<GuardRecoveryStatus, String> {
    Ok(state.guard_coordinator.recovery_status())
}

/// 重试未完成事务恢复；成功后按需启动唯一的 Guard 轮询任务。
#[tauri::command]
#[specta::specta]
pub fn guard_retry_recovery(state: State<'_, AppState>) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    match recover_pending_transactions(&state.paths) {
        Ok(()) => {
            state.guard_coordinator.clear_recovery();
            if state.guard_coordinator.claim_poll_start() {
                tauri::async_runtime::spawn(super::poll_loop(
                    state.config_store.clone(),
                    state.paths.clone(),
                    state.guard_coordinator.clone(),
                ));
            }
            Ok(())
        }
        Err(_) => {
            state
                .guard_coordinator
                .mark_recovery_blocked("recovery_failed");
            Err("recovery_failed".to_string())
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    if enabled {
        let files = load_files(&state.config_store)?;
        let schema = load_schema(&state.config_store)?;
        validate_configuration(&state.paths, &files, &schema)?;
    }
    state.config_store.update_launcher(|config| {
        config.codex_guard.enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_value(
    state: State<'_, AppState>,
    id: String,
    value: serde_json::Value,
) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    let schema = load_schema(&state.config_store)?;
    let p = find_param(&schema, &id)?;
    let type_ok = match p.value_type.as_str() {
        "bool" => value.is_boolean(),
        "int" => value.as_i64().is_some(),
        "string" | "text" => value.is_string(),
        other => {
            return Err(trf(
                "Parameter type {type} is not editable",
                &[("type", other.to_string())],
            ))
        }
    };
    if !type_ok {
        return Err(tr("Value type mismatch"));
    }
    state.config_store.update_launcher(|config| {
        let st = config.codex_guard.params.entry(id).or_default();
        if st.locked {
            return Err(tr("Parameter is locked; unlock it before modifying"));
        }
        st.value = Some(value);
        Ok(())
    })
}

fn guard_apply_inner(state: &AppState, id: String) -> Result<(), String> {
    let schema = load_schema(&state.config_store)?;
    let files = load_files(&state.config_store)?;
    validate_configuration(&state.paths, &files, &schema)?;
    let p = find_param(&schema, &id)?;
    let format = find_format(&files, &p.file)?;
    state.config_store.update_launcher(|config| {
        let expected = expected_of(&p, config.codex_guard.params.get(&id));
        execute_single_plan(&state.paths, &p, format, &expected)?;
        let st = config.codex_guard.params.entry(id).or_default();
        st.applied = true;
        st.last_checked = Some(now_secs());
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_apply(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    guard_apply_inner(&state, id)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_applied(
    state: State<'_, AppState>,
    id: String,
    applied: bool,
) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    if applied {
        return guard_apply_inner(&state, id);
    }
    // 禁用只取消看守，不回滚已写入 ~/.codex/ 的值（与删除参数的语义一致）
    state.config_store.update_launcher(|config| {
        let st = config.codex_guard.params.entry(id).or_default();
        if st.locked {
            return Err(tr("Unlock the parameter before disabling it"));
        }
        st.applied = false;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_locked(
    state: State<'_, AppState>,
    id: String,
    locked: bool,
) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    let schema = load_schema(&state.config_store)?;
    let files = load_files(&state.config_store)?;
    validate_configuration(&state.paths, &files, &schema)?;
    let p = find_param(&schema, &id)?;
    let format = find_format(&files, &p.file)?;
    state.config_store.update_launcher(|config| {
        if locked
            && !config
                .codex_guard
                .params
                .get(&id)
                .is_some_and(|state| state.applied)
        {
            return Err(tr("Apply the parameter before locking it"));
        }
        if locked {
            // 锁定即校验一次：已漂移就当场恢复
            let expected = expected_of(&p, config.codex_guard.params.get(&id));
            let c = check(&state.paths, &p, format, &expected);
            if c.status == "error" {
                return Err(c.error.unwrap_or_else(|| tr("Guard validation failed")));
            }
            let st = config.codex_guard.params.entry(id.clone()).or_default();
            st.last_checked = Some(now_secs());
            if c.status == "drift" || c.status == "missing" {
                execute_single_plan(&state.paths, &p, format, &expected)?;
                st.last_restored = Some(now_secs());
            }
        }
        let st = config.codex_guard.params.entry(id).or_default();
        st.locked = locked;
        Ok(())
    })
}

// ============ 自定义参数管理 ============

#[tauri::command]
#[specta::specta]
pub fn guard_add_custom_param(
    state: State<'_, AppState>,
    mut param: GuardParam,
    file_id: String,
) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    let files = load_files(&state.config_store)?;
    let f = find_file(&files, &file_id)
        .ok_or_else(|| trf("Target file not found: {id}", &[("id", file_id.clone())]))?;

    param.id = normalize_custom_id(&param.id);
    param.custom = true;
    param.file = f.file.clone();
    validate_param_fields(&param)?;
    validate_param_for_file(&param, f.format)?;

    let mut candidate_schema = load_schema(&state.config_store)?;
    if let Some(slot) = candidate_schema.iter_mut().find(|p| p.id == param.id) {
        *slot = param.clone();
    } else {
        candidate_schema.push(param.clone());
    }
    validate_configuration(&state.paths, &files, &candidate_schema)?;

    update_disk_schema(&state.config_store, |disk| {
        if let Some(slot) = disk.iter_mut().find(|p| p.id == param.id) {
            *slot = param;
        } else {
            disk.push(param);
        }
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_remove_custom_param(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    let normalized = normalize_custom_id(&id);

    let files = load_files(&state.config_store)?;
    let mut candidate_schema = load_schema(&state.config_store)?;
    let before = candidate_schema.len();
    candidate_schema.retain(|param| param.id != normalized);
    if candidate_schema.len() == before {
        return Err(trf(
            "Custom parameter not found: {id}",
            &[("id", normalized.clone())],
        ));
    }
    validate_configuration(&state.paths, &files, &candidate_schema)?;

    update_disk_schema(&state.config_store, |disk| {
        let disk_before = disk.len();
        disk.retain(|p| p.id != normalized);
        if disk.len() == disk_before {
            return Err(trf(
                "Custom parameter not found: {id}",
                &[("id", normalized.clone())],
            ));
        }
        Ok(())
    })?;

    // 同时清理配置里的状态，但保留已写入 codex 文件的值（不回滚，与 ADR 一致）
    state.config_store.update_launcher(|config| {
        config.codex_guard.params.remove(&normalized);
        Ok(())
    })?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn guard_get_schema_file_path(state: State<'_, AppState>) -> Result<String, String> {
    let _write = state.guard_coordinator.try_write()?;
    ensure_schema_file(&state.config_store)?;
    Ok(schema_file_path(&state.paths).to_string_lossy().to_string())
}

// ============ 文件管理命令 ============

#[tauri::command]
#[specta::specta]
pub fn guard_get_files(state: State<'_, AppState>) -> Result<Vec<GuardFile>, String> {
    let files = load_files(&state.config_store)?;
    let schema = load_schema(&state.config_store)?;
    validate_configuration(&state.paths, &files, &schema)?;
    Ok(files)
}

#[tauri::command]
#[specta::specta]
pub fn guard_add_file(
    state: State<'_, AppState>,
    name: String,
    file: String,
    format: GuardFileFormat,
) -> Result<GuardFile, String> {
    let _write = state.guard_coordinator.try_write()?;
    // 从 name 推导 id slug（简单处理：非字母数字替换为 -，去首尾 -，小写）
    let slug = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        return Err(tr("File name must contain at least one letter or digit"));
    }
    let id = normalize_custom_id(&slug);

    let trimmed_file = normalize_relative_path(file.trim()).map_err(|error| error.to_string())?;
    let gf = GuardFile {
        id: id.clone(),
        name: name.trim().to_string(),
        file: trimmed_file.clone(),
        format,
        builtin: false,
        detection: None,
    };
    validate_guard_file(&gf)?;
    let schema = load_schema(&state.config_store)?;
    update_files(&state.config_store, |files| {
        // id 与路径冲突检查（同路径会让参数在两个分组里重复显示）
        if files.iter().any(|file| file.id == id) {
            return Err(trf(
                "A file with the same name already exists: {name}",
                &[("name", name.clone())],
            ));
        }
        if files.iter().any(|file| file.file == trimmed_file) {
            return Err(trf(
                "Path already in guard list: {path}",
                &[("path", trimmed_file.clone())],
            ));
        }
        files.push(gf.clone());
        validate_configuration(&state.paths, files, &schema)?;
        Ok(gf)
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_update_file(
    state: State<'_, AppState>,
    id: String,
    name: String,
    file: String,
) -> Result<GuardFile, String> {
    let _write = state.guard_coordinator.try_write()?;
    let files = load_files(&state.config_store)?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;

    let old_file = files[idx].file.clone();
    let new_file = normalize_relative_path(file.trim()).map_err(|error| error.to_string())?;

    if files[idx].builtin && old_file != new_file {
        return Err(tr("Built-in file paths cannot be changed"));
    }

    if old_file != new_file && files.iter().any(|f| f.id != id && f.file == new_file) {
        return Err(trf(
            "Path already in guard list: {path}",
            &[("path", new_file.clone())],
        ));
    }

    let mut updated = files[idx].clone();
    updated.name = name.trim().to_string();
    updated.file = new_file.clone();
    if old_file != new_file {
        updated.detection = None;
    }
    validate_guard_file(&updated)?;

    let mut candidate_files = files.clone();
    candidate_files[idx] = updated.clone();
    let mut candidate_schema = load_schema(&state.config_store)?;
    if old_file != new_file {
        for param in &mut candidate_schema {
            if param.custom && param.file == old_file {
                param.file = new_file.clone();
            }
        }
    }
    validate_configuration(&state.paths, &candidate_files, &candidate_schema)?;

    // 如果是自定义参数的归属文件，路径变了参数的 file 也要跟着变
    // schema 中该文件路径下的自定义参数需要更新 file 字段
    if old_file != new_file {
        update_disk_schema(&state.config_store, |disk| {
            for param in disk {
                if param.custom && param.file == old_file {
                    param.file = new_file.clone();
                }
            }
            Ok(())
        })?;
    }
    update_files(&state.config_store, |current| {
        if current
            .iter()
            .any(|file| file.id != id && file.file == new_file)
        {
            return Err(trf(
                "Path already in guard list: {path}",
                &[("path", new_file.clone())],
            ));
        }
        let slot = current
            .iter_mut()
            .find(|file| file.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
        *slot = updated.clone();
        Ok(updated)
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_remove_file(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _write = state.guard_coordinator.try_write()?;
    let files = load_files(&state.config_store)?;
    let idx = files
        .iter()
        .position(|file| file.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
    if files[idx].builtin {
        return Err(tr("Built-in files cannot be removed"));
    }
    let target_file = files[idx].file.clone();
    let candidate_files = files
        .iter()
        .filter(|file| file.id != id)
        .cloned()
        .collect::<Vec<_>>();
    let schema = load_schema(&state.config_store)?;
    let candidate_schema = schema
        .into_iter()
        .filter(|param| !(param.custom && param.file == target_file))
        .collect::<Vec<_>>();
    validate_configuration(&state.paths, &candidate_files, &candidate_schema)?;

    update_files(&state.config_store, |current| {
        let idx = current
            .iter()
            .position(|file| file.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
        if current[idx].builtin {
            return Err(tr("Built-in files cannot be removed"));
        }
        current.remove(idx);
        Ok(())
    })?;

    // 清理该文件下的所有自定义参数（schema + 状态）
    // 不回滚已写入 codex 的值（与 ADR 一致）
    let removed_ids = update_disk_schema(&state.config_store, |disk| {
        let removed_ids: Vec<String> = disk
            .iter()
            .filter(|param| param.custom && param.file == target_file)
            .map(|param| param.id.clone())
            .collect();
        disk.retain(|param| !(param.custom && param.file == target_file));
        Ok(removed_ids)
    })?;

    state.config_store.update_launcher(|config| {
        for id in &removed_ids {
            config.codex_guard.params.remove(id);
        }
        Ok(())
    })?;

    Ok(())
}

// ============ 路径检测 ============

/// 检测文件实际路径并落盘记录；之后直接读记录，不重复扫盘
#[tauri::command]
#[specta::specta]
pub fn guard_detect_file(state: State<'_, AppState>, id: String) -> Result<GuardFile, String> {
    let _write = state.guard_coordinator.try_write()?;
    let files = load_files(&state.config_store)?;
    let schema = load_schema(&state.config_store)?;
    validate_configuration(&state.paths, &files, &schema)?;
    update_files(&state.config_store, |files| {
        let file = files
            .iter_mut()
            .find(|file| file.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
        file.detection = Some(DetectRecord {
            path: detect_file_path(&state.paths, &file.file),
            at: now_secs(),
        });
        Ok(file.clone())
    })
}

/// 把文件选择器选中的绝对路径换算为相对 ~/.codex 的路径（越界拒绝）
#[tauri::command]
#[specta::specta]
pub fn guard_relativize_picked_path(
    state: State<'_, AppState>,
    abs_path: String,
) -> Result<String, String> {
    let home = state.paths.codex_root();
    let rel = std::path::Path::new(&abs_path)
        .strip_prefix(home)
        .map_err(|_| tr("Selected file must be inside ~/.codex"))?;
    let rel = normalize_relative_path(&rel.to_string_lossy()).map_err(|error| error.to_string())?;
    validate_target_path(&state.paths, &rel).map_err(|error| error.to_string())
}
