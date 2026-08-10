//! Tauri 命令：前端调用的全部入口（参数管理 / 自定义参数 / 看守文件管理 / 路径检测）

use crate::i18n::{tr, trf};
use crate::AppState;
use tauri::State;

use super::engine::{apply, check, expected_of};
use super::files::{detect_file_path, find_file, load_files, update_files};
use super::schema::{
    ensure_schema_file, load_schema, schema_file_path, update_disk_schema,
};
use super::validate::{
    normalize_custom_id, validate_file_path, validate_guard_file, validate_param_fields,
};
use super::view::{build_view, GuardView};
use super::{now_secs, DetectRecord, GuardFile, GuardParam};

fn find_param(schema: &[GuardParam], id: &str) -> Result<GuardParam, String> {
    schema
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| trf("Parameter not found in schema: {id}", &[("id", id.to_string())]))
}

#[tauri::command]
#[specta::specta]
pub fn guard_get_view(state: State<'_, AppState>) -> Result<GuardView, String> {
    build_view(&state.config_store, &state.paths)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    state.config_store.update_launcher(|config| {
        config.codex_guard.enabled = enabled;
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_value(state: State<'_, AppState>, id: String, value: serde_json::Value) -> Result<(), String> {
    let schema = load_schema(&state.config_store)?;
    let p = find_param(&schema, &id)?;
    let type_ok = match p.value_type.as_str() {
        "bool" => value.is_boolean(),
        "int" => value.as_i64().is_some(),
        "string" | "text" => value.is_string(),
        other => return Err(trf("Parameter type {type} is not editable", &[("type", other.to_string())])),
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
    let p = find_param(&schema, &id)?;
    state.config_store.update_launcher(|config| {
        let expected = expected_of(&p, config.codex_guard.params.get(&id));
        apply(&state.paths, &p, &expected)?;
        let st = config.codex_guard.params.entry(id).or_default();
        st.applied = true;
        st.last_checked = Some(now_secs());
        Ok(())
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_apply(state: State<'_, AppState>, id: String) -> Result<(), String> {
    guard_apply_inner(&state, id)
}

#[tauri::command]
#[specta::specta]
pub fn guard_set_applied(state: State<'_, AppState>, id: String, applied: bool) -> Result<(), String> {
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
pub fn guard_set_locked(state: State<'_, AppState>, id: String, locked: bool) -> Result<(), String> {
    let schema = load_schema(&state.config_store)?;
    let p = find_param(&schema, &id)?;
    state.config_store.update_launcher(|config| {
        let st = config.codex_guard.params.entry(id.clone()).or_default();
        if locked && !st.applied {
            return Err(tr("Apply the parameter before locking it"));
        }
        st.locked = locked;
        if locked {
            // 锁定即校验一次：已漂移就当场恢复
            let expected = expected_of(&p, config.codex_guard.params.get(&id));
            let c = check(&state.paths, &p, &expected);
            let st = config.codex_guard.params.entry(id.clone()).or_default();
            st.last_checked = Some(now_secs());
            if c.status == "drift" || c.status == "missing" {
                apply(&state.paths, &p, &expected)?;
                st.last_restored = Some(now_secs());
            }
        }
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
    let files = load_files(&state.config_store)?;
    let f = find_file(&files, &file_id)
        .ok_or_else(|| trf("Target file not found: {id}", &[("id", file_id.clone())]))?;

    param.id = normalize_custom_id(&param.id);
    param.custom = true;
    param.file = f.file.clone();
    validate_param_fields(&param)?;

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
    let normalized = normalize_custom_id(&id);

    update_disk_schema(&state.config_store, |disk| {
        let before = disk.len();
        disk.retain(|p| p.id != normalized);
        if disk.len() == before {
            return Err(trf("Custom parameter not found: {id}", &[("id", normalized.clone())]));
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
    ensure_schema_file(&state.config_store)?;
    Ok(schema_file_path(&state.paths).to_string_lossy().to_string())
}

// ============ 文件管理命令 ============

#[tauri::command]
#[specta::specta]
pub fn guard_get_files(state: State<'_, AppState>) -> Result<Vec<GuardFile>, String> {
    load_files(&state.config_store)
}

#[tauri::command]
#[specta::specta]
pub fn guard_add_file(state: State<'_, AppState>, name: String, file: String, format: String) -> Result<GuardFile, String> {
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

    let trimmed_file = file.trim().to_string();
    let gf = GuardFile {
        id: id.clone(),
        name: name.trim().to_string(),
        file: trimmed_file.clone(),
        format,
        builtin: false,
        detection: None,
    };
    validate_guard_file(&gf)?;
    update_files(&state.config_store, |files| {
        // id 与路径冲突检查（同路径会让参数在两个分组里重复显示）
        if files.iter().any(|file| file.id == id) {
            return Err(trf("A file with the same name already exists: {name}", &[("name", name.clone())]));
        }
        if files.iter().any(|file| file.file == trimmed_file) {
            return Err(trf("Path already in guard list: {path}", &[("path", trimmed_file.clone())]));
        }
        files.push(gf.clone());
        Ok(gf)
    })
}

#[tauri::command]
#[specta::specta]
pub fn guard_update_file(state: State<'_, AppState>, id: String, name: String, file: String) -> Result<GuardFile, String> {
    let files = load_files(&state.config_store)?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;

    let old_file = files[idx].file.clone();
    let new_file = file.trim().to_string();

    if old_file != new_file && files.iter().any(|f| f.id != id && f.file == new_file) {
        return Err(trf("Path already in guard list: {path}", &[("path", new_file.clone())]));
    }

    let mut updated = files[idx].clone();
    updated.name = name.trim().to_string();
    updated.file = new_file.clone();
    if old_file != new_file {
        updated.detection = None;
    }
    validate_guard_file(&updated)?;

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
        if current.iter().any(|file| file.id != id && file.file == new_file) {
            return Err(trf("Path already in guard list: {path}", &[("path", new_file.clone())]));
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
    let target_file = update_files(&state.config_store, |files| {
        let idx = files
            .iter()
            .position(|file| file.id == id)
            .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
        if files[idx].builtin {
            return Err(tr("Built-in files cannot be removed"));
        }
        Ok(files.remove(idx).file)
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
pub fn guard_relativize_picked_path(state: State<'_, AppState>, abs_path: String) -> Result<String, String> {
    let home = state.paths.codex_root();
    let rel = std::path::Path::new(&abs_path)
        .strip_prefix(home)
        .map_err(|_| tr("Selected file must be inside ~/.codex"))?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    validate_file_path(&rel)?;
    Ok(rel)
}
