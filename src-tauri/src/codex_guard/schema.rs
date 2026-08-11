//! schema：内置 + 磁盘托管参数的加载、合并与按界面语言取值

use std::sync::OnceLock;

use crate::config::ConfigStore;

use super::ownership::validate_param_ids;
use super::{AppPaths, GuardFile, GuardParam};

const BUILTIN_SCHEMA: &str = include_str!("guard_schema.json");

/// 内置 schema 随二进制发布、永不变化，只解析一次；此前每次 load_schema
/// （每个 view/poll/写命令至少一次）都重新解析 16KB JSON。
fn builtin_schema() -> &'static [GuardParam] {
    static BUILTIN: OnceLock<Vec<GuardParam>> = OnceLock::new();
    BUILTIN
        .get_or_init(|| serde_json::from_str(BUILTIN_SCHEMA).expect("内置 guard schema 必须可解析"))
}

/// 按界面语言挑文案：en 且有英文资源时用英文，否则用原文（纯函数，便于测试）
pub(crate) fn pick_i18n<'a>(primary: &'a str, en: &'a str, lang: &str) -> &'a str {
    if lang == "en" && !en.is_empty() {
        en
    } else {
        primary
    }
}

/// 按界面语言挑 default 值：en 且有英文内容时用英文，否则用原文（纯函数，便于测试）
pub(crate) fn default_for_lang<'a>(p: &'a GuardParam, lang: &str) -> &'a serde_json::Value {
    if lang == "en" && !p.default_en.is_null() {
        &p.default_en
    } else {
        &p.default
    }
}

pub(crate) fn schema_file_path(paths: &AppPaths) -> std::path::PathBuf {
    paths.guard_schema_file()
}

/// 加载 schema：磁盘同 id 覆盖内置（用户定制内置参数），磁盘独有条目保留；
/// 例外：label_en/description_en/default_en 是随二进制发布的英文资源、不算用户数据，
/// 始终以内置为准——否则旧版本释放到磁盘的中文副本会永久滞留（i18n 实机踩坑）
pub(crate) fn load_schema(store: &ConfigStore) -> Result<Vec<GuardParam>, String> {
    let disk = store.load_guard_schema()?;
    validate_param_ids(&disk).map_err(|error| error.to_string())?;
    let mut merged = merge_schema(builtin_schema().to_vec(), disk);
    let files = super::files::load_files(store)?;
    for param in &mut merged {
        if param.file_id.is_empty() {
            param.file_id = stable_file_id_for_path(&param.file, &files);
        }
    }
    Ok(merged)
}

/// 为缺 `file_id` 的旧参数补齐稳定文件 ID。必须以实际看守文件列表为准：
/// 自定义文件的 ID 是 `custom.<slug>`，凭路径猜出的 `path:<路径>` 永不匹配，
/// 会让所有权预检把该参数判成 UnknownFile 并阻断整个 Guard 域。
fn stable_file_id_for_path(path: &str, files: &[GuardFile]) -> String {
    if let Some(file) = files.iter().find(|file| file.file == path) {
        return file.id.clone();
    }
    format!("path:{path}")
}

/// 合并内置与磁盘 schema（纯函数，便于测试）
fn merge_schema(builtin: Vec<GuardParam>, disk: Vec<GuardParam>) -> Vec<GuardParam> {
    let mut merged = builtin;
    for mut d in disk {
        if let Some(slot) = merged.iter().position(|m| m.id == d.id) {
            // 英文资源以内置条目为准，其余字段尊重磁盘定制
            d.label_en = merged[slot].label_en.clone();
            d.description_en = merged[slot].description_en.clone();
            d.default_en = merged[slot].default_en.clone();
            merged[slot] = d;
        } else {
            merged.push(d); // 磁盘独有条目保留
        }
    }
    merged
}

pub(crate) fn update_disk_schema<R>(
    store: &ConfigStore,
    update: impl FnOnce(&mut Vec<GuardParam>) -> Result<R, String>,
) -> Result<R, String> {
    store.update_guard_schema(update)
}

pub(crate) fn ensure_schema_file(store: &ConfigStore) -> Result<(), String> {
    store.ensure_guard_schema_file(BUILTIN_SCHEMA.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::GuardFileFormat;

    #[test]
    fn builtin_schema_parses() {
        let v: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        assert_eq!(v.len(), 11);
    }

    /// A legacy custom parameter on a user-added Guard file carries no `file_id`.
    /// Deriving one from the path alone yields `path:<path>`, which never matches the
    /// file's real `custom.<slug>` id — ownership preflight then reports UnknownFile
    /// and every view, poll, and batch command fails with no way to recover.
    #[test]
    fn backfilled_file_id_resolves_to_the_real_custom_guard_file() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.launcher_root()).unwrap();
        let store = ConfigStore::new(paths.clone());

        let custom_file = GuardFile {
            id: "custom.my-config".to_string(),
            name: "My config".to_string(),
            file: "myconfig.toml".to_string(),
            format: GuardFileFormat::Toml,
            builtin: false,
            detection: None,
        };
        store
            .update_launcher(|config| {
                let mut files = super::super::files::builtin_files();
                files.push(custom_file.clone());
                config.codex_guard.files = files;
                Ok(())
            })
            .unwrap();
        store
            .update_guard_schema(|disk| {
                disk.push(
                    serde_json::from_value::<GuardParam>(serde_json::json!({
                        "id": "custom.legacy",
                        "label": "Legacy",
                        "file": "myconfig.toml",
                        "apply_mode": "toml_key",
                        "path": "alpha",
                        "custom": true,
                    }))
                    .unwrap(),
                );
                Ok(())
            })
            .unwrap();

        let schema = load_schema(&store).unwrap();
        let param = schema
            .iter()
            .find(|param| param.id == "custom.legacy")
            .expect("custom parameter survives merge");
        assert_eq!(
            param.file_id, "custom.my-config",
            "backfill must resolve the real Guard file id, not guess from the path"
        );

        let files = super::super::files::load_files(&store).unwrap();
        super::super::ownership::validate_ownership(&paths, &files, &schema)
            .expect("ownership preflight must accept a backfilled custom parameter");
    }

    #[test]
    fn builtin_schema_has_english_labels() {
        // 内置参数必须带英文资源，否则英文界面落回中文原文
        let v: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        for p in &v {
            assert!(!p.label_en.is_empty(), "{} 缺 label_en", p.id);
            assert!(!p.description_en.is_empty(), "{} 缺 description_en", p.id);
        }
    }

    #[test]
    fn pick_i18n_selects_by_language() {
        // en 有英文资源 → 英文；无英文资源（自定义参数）→ 落回原文；zh → 原文
        assert_eq!(pick_i18n("中文", "English", "en"), "English");
        assert_eq!(pick_i18n("自定义", "", "en"), "自定义");
        assert_eq!(pick_i18n("中文", "English", "zh-CN"), "中文");
    }

    #[test]
    fn merge_schema_keeps_builtin_english_resources() {
        // 回归：旧版磁盘副本无 label_en，同 id 覆盖内置时不得盖掉英文资源
        let json = |id: &str, label: &str, label_en: &str, default_en: serde_json::Value| {
            serde_json::from_value::<GuardParam>(serde_json::json!({
                "id": id, "label": label, "label_en": label_en,
                "file": "config.toml", "apply_mode": "toml_key",
                "default_en": default_en,
            }))
            .unwrap()
        };
        let builtin = vec![json(
            "a",
            "内置中文",
            "Builtin EN",
            serde_json::json!("EN content"),
        )];
        let disk = vec![
            // 同 id 覆盖：label 尊重磁盘，英文资源（含 default_en）以内置为准
            json("a", "用户改过的中文", "", serde_json::Value::Null),
            json("custom.x", "自定义", "", serde_json::Value::Null), // 磁盘独有：保留
        ];
        let merged = merge_schema(builtin, disk);
        assert_eq!(merged.len(), 2);
        let a = merged.iter().find(|p| p.id == "a").unwrap();
        assert_eq!(a.label, "用户改过的中文"); // 磁盘定制生效
        assert_eq!(a.label_en, "Builtin EN"); // 英文资源以内置为准
        assert_eq!(a.default_en, serde_json::json!("EN content"));
        assert!(merged.iter().any(|p| p.id == "custom.x"));
    }

    #[test]
    fn default_for_lang_selects_content_by_language() {
        let p = |default_en: serde_json::Value| GuardParam {
            id: "x".into(),
            label: "中文".into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: "f".into(),
            file_id: String::new(),
            group_id: None,
            apply_mode: "file_overwrite".into(),
            path: String::new(),
            value_type: "text".into(),
            default: serde_json::json!("中文内容"),
            default_en,
            custom: false,
        };
        // en + 有英文内容 → 英文；en + 无英文内容 → 原文；zh → 原文
        let with_en = p(serde_json::json!("English content"));
        assert_eq!(
            default_for_lang(&with_en, "en"),
            &serde_json::json!("English content")
        );
        assert_eq!(
            default_for_lang(&with_en, "zh-CN"),
            &serde_json::json!("中文内容")
        );
        let without_en = p(serde_json::Value::Null);
        assert_eq!(
            default_for_lang(&without_en, "en"),
            &serde_json::json!("中文内容")
        );
    }

    #[test]
    fn builtin_schema_has_no_custom_flag() {
        let builtin: Vec<GuardParam> = serde_json::from_str(BUILTIN_SCHEMA).unwrap();
        for p in &builtin {
            assert!(!p.custom, "内置参数 {} 不应有 custom=true", p.id);
        }
    }
}
