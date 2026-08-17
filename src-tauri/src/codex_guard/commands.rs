//! Tauri 命令：前端调用的全部入口（参数管理 / 自定义参数 / 看守文件管理 / 路径检测）

use crate::config;
use crate::i18n::{tr, trf};

use super::engine::{apply, check, expected_of};
use super::files::{detect_file_path, find_file, load_files, save_files};
use super::schema::{load_disk_schema, load_schema, save_disk_schema, schema_file_path};
use super::validate::{
    normalize_custom_id, validate_file_path, validate_guard_file, validate_param_fields,
};
use super::view::{build_view, GuardView};
use super::{codex_home, now_secs, DetectRecord, GuardFile, GuardParam};

fn find_param(schema: &[GuardParam], id: &str) -> Result<GuardParam, String> {
    schema
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or_else(|| {
            crate::logging::error("看守: 查找参数", id);
            trf("Parameter not found in schema: {id}", &[("id", id.to_string())])
        })
}

#[tauri::command]
pub fn guard_get_view() -> Result<GuardView, String> {
    build_view()
}

#[tauri::command]
pub fn guard_set_enabled(enabled: bool) -> Result<(), String> {
    let mut cfg = config::load_config()?;
    cfg.codex_guard.enabled = enabled;
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_value(id: String, value: serde_json::Value) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let type_ok = match p.value_type.as_str() {
        "bool" => value.is_boolean(),
        "int" => value.as_i64().is_some(),
        "string" | "text" => value.is_string(),
        other => {
            let err = trf("Parameter type {type} is not editable", &[("type", other.to_string())]);
            crate::logging::warn("看守: 修改参数", &err);
            return Err(err);
        }
    };
    if !type_ok {
        let err = tr("Value type mismatch");
        crate::logging::warn("看守: 修改参数", &err);
        return Err(err);
    }
    let mut cfg = config::load_config()?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    if st.locked {
        let err = tr("Parameter is locked; unlock it before modifying");
        crate::logging::warn("看守: 修改参数", &err);
        return Err(err);
    }
    st.value = Some(value);
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_apply(id: String) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let mut cfg = config::load_config()?;
    let expected = expected_of(&p, cfg.codex_guard.params.get(&id));
    apply(&p, &expected)?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    st.applied = true;
    st.last_checked = Some(now_secs());
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_applied(id: String, applied: bool) -> Result<(), String> {
    if applied {
        return guard_apply(id);
    }
    // 禁用只取消看守，不回滚已写入 ~/.codex/ 的值（与删除参数的语义一致）
    let mut cfg = config::load_config()?;
    let st = cfg.codex_guard.params.entry(id).or_default();
    if st.locked {
        let err = tr("Unlock the parameter before disabling it");
        crate::logging::warn("看守: 停用参数", &err);
        return Err(err);
    }
    st.applied = false;
    config::save_config(&cfg)
}

#[tauri::command]
pub fn guard_set_locked(id: String, locked: bool) -> Result<(), String> {
    let schema = load_schema();
    let p = find_param(&schema, &id)?;
    let mut cfg = config::load_config()?;
    {
        let st = cfg.codex_guard.params.entry(id.clone()).or_default();
        if locked && !st.applied {
            let err = tr("Apply the parameter before locking it");
            crate::logging::warn("看守: 锁定参数", &err);
            return Err(err);
        }
        st.locked = locked;
    }
    if locked {
        // 锁定即校验一次：已漂移就当场恢复
        let expected = expected_of(&p, cfg.codex_guard.params.get(&id));
        let c = check(&p, &expected);
        let st = cfg.codex_guard.params.entry(id).or_default();
        st.last_checked = Some(now_secs());
        if c.status == "drift" || c.status == "missing" {
            apply(&p, &expected)?;
            st.last_restored = Some(now_secs());
        }
    }
    config::save_config(&cfg)
}

// ============ 自定义参数管理 ============

#[tauri::command]
pub fn guard_add_custom_param(
    mut param: GuardParam,
    file_id: String,
) -> Result<(), String> {
    let files = load_files()?;
    let f = find_file(&files, &file_id)
        .ok_or_else(|| {
            let err = trf("Target file not found: {id}", &[("id", file_id.clone())]);
            crate::logging::error("看守: 添加自定义参数", &err);
            err
        })?;

    param.id = normalize_custom_id(&param.id);
    param.custom = true;
    param.file = f.file.clone();
    validate_param_fields(&param)?;

    let mut disk = load_disk_schema().unwrap_or_default();
    if let Some(slot) = disk.iter_mut().find(|p| p.id == param.id) {
        *slot = param;
    } else {
        disk.push(param);
    }
    save_disk_schema(&disk)
}

#[tauri::command]
pub fn guard_remove_custom_param(id: String) -> Result<(), String> {
    let normalized = normalize_custom_id(&id);

    let mut disk = load_disk_schema().unwrap_or_default();
    let before = disk.len();
    disk.retain(|p| p.id != normalized);
    if disk.len() == before {
        let err = trf("Custom parameter not found: {id}", &[("id", normalized.clone())]);
        crate::logging::error("看守: 删除自定义参数", &err);
        return Err(err);
    }
    save_disk_schema(&disk)?;

    // 同时清理配置里的状态，但保留已写入 codex 文件的值（不回滚，与 ADR 一致）
    let mut cfg = config::load_config()?;
    cfg.codex_guard.params.remove(&normalized);
    config::save_config(&cfg)?;

    Ok(())
}

#[tauri::command]
pub fn guard_get_schema_file_path() -> Result<String, String> {
    schema_file_path().map(|p| p.to_string_lossy().to_string())
}

// ============ 文件管理命令 ============

#[tauri::command]
pub fn guard_get_files() -> Result<Vec<GuardFile>, String> {
    load_files()
}

#[tauri::command]
pub fn guard_add_file(name: String, file: String, format: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;

    // 从 name 推导 id slug（简单处理：非字母数字替换为 -，去首尾 -，小写）
    let slug = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        let err = tr("File name must contain at least one letter or digit");
        crate::logging::warn("看守: 添加文件", &err);
        return Err(err);
    }
    let id = normalize_custom_id(&slug);

    // id 与路径冲突检查（同路径会让参数在两个分组里重复显示）
    if files.iter().any(|f| f.id == id) {
        let err = trf("A file with the same name already exists: {name}", &[("name", name.clone())]);
        crate::logging::warn("看守: 添加文件", &err);
        return Err(err);
    }
    let trimmed_file = file.trim().to_string();
    if files.iter().any(|f| f.file == trimmed_file) {
        let err = trf("Path already in guard list: {path}", &[("path", trimmed_file.clone())]);
        crate::logging::warn("看守: 添加文件", &err);
        return Err(err);
    }

    let gf = GuardFile {
        id: id.clone(),
        name: name.trim().to_string(),
        file: trimmed_file,
        format,
        builtin: false,
        detection: None,
    };
    validate_guard_file(&gf)?;

    files.push(gf.clone());
    save_files(&files)?;

    Ok(gf)
}

#[tauri::command]
pub fn guard_update_file(id: String, name: String, file: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| {
            let err = trf("File not found: {id}", &[("id", id.clone())]);
            crate::logging::error("看守: 更新文件", &err);
            err
        })?;

    let old_file = files[idx].file.clone();
    let new_file = file.trim().to_string();

    if old_file != new_file && files.iter().any(|f| f.id != id && f.file == new_file) {
        let err = trf("Path already in guard list: {path}", &[("path", new_file.clone())]);
        crate::logging::warn("看守: 更新文件", &err);
        return Err(err);
    }

    let f = &mut files[idx];
    f.name = name.trim().to_string();
    f.file = new_file.clone();
    if old_file != new_file {
        // 路径变了，旧检测记录作废（下次打开设置页会重新检测一次）
        f.detection = None;
    }
    validate_guard_file(f)?;

    // 如果是自定义参数的归属文件，路径变了参数的 file 也要跟着变
    // schema 中该文件路径下的自定义参数需要更新 file 字段
    if old_file != new_file {
        let mut disk = load_disk_schema().unwrap_or_default();
        let mut changed = false;
        for p in disk.iter_mut() {
            if p.custom && p.file == old_file {
                p.file = new_file.clone();
                changed = true;
            }
        }
        if changed {
            save_disk_schema(&disk)?;
        }
    }

    save_files(&files)?;
    Ok(files[idx].clone())
}

#[tauri::command]
pub fn guard_remove_file(id: String) -> Result<(), String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| {
            let err = trf("File not found: {id}", &[("id", id.clone())]);
            crate::logging::error("看守: 删除文件", &err);
            err
        })?;

    if files[idx].builtin {
        let err = tr("Built-in files cannot be removed");
        crate::logging::warn("看守: 删除文件", &err);
        return Err(err);
    }

    let target_file = files[idx].file.clone();
    files.remove(idx);
    save_files(&files)?;

    // 清理该文件下的所有自定义参数（schema + 状态）
    // 不回滚已写入 codex 的值（与 ADR 一致）
    let mut disk = load_disk_schema().unwrap_or_default();
    // 先收集待删参数的 id 再删 schema（删完就查不到了），用于清理配置里的状态
    let removed_ids: Vec<String> = disk
        .iter()
        .filter(|p| p.custom && p.file == target_file)
        .map(|p| p.id.clone())
        .collect();
    disk.retain(|p| !(p.custom && p.file == target_file));
    if !removed_ids.is_empty() {
        save_disk_schema(&disk)?;
    }

    let mut cfg = config::load_config()?;
    for pid in &removed_ids {
        cfg.codex_guard.params.remove(pid);
    }
    config::save_config(&cfg)?;

    Ok(())
}

// ============ 路径检测 ============

/// 检测文件实际路径并落盘记录；之后直接读记录，不重复扫盘
#[tauri::command]
pub fn guard_detect_file(id: String) -> Result<GuardFile, String> {
    let mut files = load_files()?;
    let idx = files
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| trf("File not found: {id}", &[("id", id.clone())]))?;
    let detected = detect_file_path(&files[idx].file);
    let f = &mut files[idx];
    f.detection = Some(DetectRecord {
        path: detected,
        at: now_secs(),
    });
    let out = f.clone();
    save_files(&files)?;
    Ok(out)
}

/// 把文件选择器选中的绝对路径换算为相对 ~/.codex 的路径（越界拒绝）
#[tauri::command]
pub fn guard_relativize_picked_path(abs_path: String) -> Result<String, String> {
    let home = codex_home()?;
    let rel = std::path::Path::new(&abs_path)
        .strip_prefix(&home)
        .map_err(|_| {
            let err = tr("Selected file must be inside ~/.codex");
            crate::logging::warn("看守: 换算选中路径", &err);
            err
        })?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    validate_file_path(&rel)?;
    Ok(rel)
}
