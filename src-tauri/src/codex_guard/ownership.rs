//! Guard 配置的路径与物理所有权预检。

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::fastctx;

use super::{AppPaths, GuardFile, GuardParam};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnershipCode {
    InvalidPath,
    NonCanonicalPath,
    PathOutsideRoot,
    PathSymlink,
    PathTargetDirectory,
    PathProbeFailed,
    InvalidParamId,
    DuplicateFileId,
    DuplicateFilePath,
    ParentFilePathConflict,
    DuplicateParamId,
    UnknownFile,
    DuplicateTomlPath,
    TomlParentChildConflict,
    TomlAbsentConflict,
    FileOverwriteConflict,
    ExternalKeyOwned,
}

impl OwnershipCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidPath => "ownership.invalid_path",
            Self::NonCanonicalPath => "ownership.non_canonical_path",
            Self::PathOutsideRoot => "ownership.path_outside_root",
            Self::PathSymlink => "ownership.path_symlink",
            Self::PathTargetDirectory => "ownership.path_target_directory",
            Self::PathProbeFailed => "ownership.path_probe_failed",
            Self::InvalidParamId => "ownership.invalid_param_id",
            Self::DuplicateFileId => "ownership.duplicate_file_id",
            Self::DuplicateFilePath => "ownership.duplicate_file_path",
            Self::ParentFilePathConflict => "ownership.parent_file_path_conflict",
            Self::DuplicateParamId => "ownership.duplicate_param_id",
            Self::UnknownFile => "ownership.unknown_file",
            Self::DuplicateTomlPath => "ownership.duplicate_toml_path",
            Self::TomlParentChildConflict => "ownership.toml_parent_child_conflict",
            Self::TomlAbsentConflict => "ownership.toml_absent_conflict",
            Self::FileOverwriteConflict => "ownership.file_overwrite_conflict",
            Self::ExternalKeyOwned => "ownership.external_key_owned",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OwnershipError {
    pub(crate) code: OwnershipCode,
}

impl OwnershipError {
    const fn new(code: OwnershipCode) -> Self {
        Self { code }
    }
}

impl fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code.as_str())
    }
}

/// 统一相对 ~/.codex 的路径拼法，不把 Path::join 的平台差异带入领域模型。
pub(crate) fn normalize_relative_path(raw: &str) -> Result<String, OwnershipError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err(OwnershipError::new(OwnershipCode::InvalidPath));
    }
    let separators = raw.replace('\\', "/");
    if separators.starts_with('/')
        || separators
            .split('/')
            .next()
            .is_some_and(|segment| segment.contains(':'))
    {
        return Err(OwnershipError::new(OwnershipCode::InvalidPath));
    }

    let mut parts = Vec::new();
    for segment in separators.split('/') {
        match segment {
            "" | "." => {}
            ".." => return Err(OwnershipError::new(OwnershipCode::InvalidPath)),
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err(OwnershipError::new(OwnershipCode::InvalidPath));
    }
    Ok(parts.join("/"))
}

/// 检查一组磁盘 schema 条目自身的 ID；在 merge_schema 吞掉重复项之前调用。
pub(crate) fn validate_param_ids(params: &[GuardParam]) -> Result<(), OwnershipError> {
    let mut ids = HashSet::new();
    for param in params {
        if !valid_param_id(&param.id) {
            return Err(OwnershipError::new(OwnershipCode::InvalidParamId));
        }
        if !ids.insert(casefold(&param.id)) {
            return Err(OwnershipError::new(OwnershipCode::DuplicateParamId));
        }
    }
    Ok(())
}

/// 在任何 Guard 写入前校验文件、参数和物理 TOML 所有权。
pub(crate) fn validate_ownership(
    paths: &AppPaths,
    files: &[GuardFile],
    params: &[GuardParam],
) -> Result<(), OwnershipError> {
    let mut ids = HashSet::new();
    let mut file_keys = HashMap::<String, (String, PathBuf)>::new();
    let mut file_paths = Vec::<(String, Vec<String>)>::new();

    for file in files {
        if !valid_param_id(&file.id) {
            return Err(OwnershipError::new(OwnershipCode::InvalidParamId));
        }
        if !ids.insert(casefold(&file.id)) {
            return Err(OwnershipError::new(OwnershipCode::DuplicateFileId));
        }
        let normalized = normalize_relative_path(&file.file)?;
        if normalized != file.file {
            return Err(OwnershipError::new(OwnershipCode::NonCanonicalPath));
        }
        let real = resolve_guard_path(paths, &normalized)?;
        let key = path_key(&normalized);
        if file_keys.contains_key(&key)
            || file_paths
                .iter()
                .any(|(_, existing)| is_path_prefix(existing, &path_parts(&normalized)))
            || file_paths
                .iter()
                .any(|(_, existing)| is_path_prefix(&path_parts(&normalized), existing))
        {
            return Err(OwnershipError::new(if file_keys.contains_key(&key) {
                OwnershipCode::DuplicateFilePath
            } else {
                OwnershipCode::ParentFilePathConflict
            }));
        }
        if file_keys.values().any(|(_, existing)| existing == &real) {
            return Err(OwnershipError::new(OwnershipCode::DuplicateFilePath));
        }
        file_keys.insert(key.clone(), (file.file.clone(), real));
        file_paths.push((key, path_parts(&normalized)));
    }

    validate_param_ids(params)?;
    let mut toml_paths = HashMap::<String, Vec<(Vec<String>, bool)>>::new();
    let mut file_modes = HashMap::<String, usize>::new();

    for param in params {
        let normalized_file = normalize_relative_path(&param.file)?;
        if normalized_file != param.file {
            return Err(OwnershipError::new(OwnershipCode::NonCanonicalPath));
        }
        let file_key = path_key(&normalized_file);
        let (_, file_real) = file_keys
            .get(&file_key)
            .ok_or_else(|| OwnershipError::new(OwnershipCode::UnknownFile))?;
        if !param.file_id.is_empty() {
            let matching_file = files
                .iter()
                .find(|file| file.id == param.effective_file_id())
                .ok_or_else(|| OwnershipError::new(OwnershipCode::UnknownFile))?;
            if matching_file.file != normalized_file {
                return Err(OwnershipError::new(OwnershipCode::UnknownFile));
            }
        }
        let _ = file_real;

        if param.apply_mode == "file_overwrite" {
            let count = file_modes.entry(file_key.clone()).or_default();
            *count += 1;
            if *count > 1 {
                return Err(OwnershipError::new(OwnershipCode::FileOverwriteConflict));
            }
        } else if file_modes.contains_key(&file_key) {
            return Err(OwnershipError::new(OwnershipCode::FileOverwriteConflict));
        }

        if matches!(param.apply_mode.as_str(), "toml_key" | "toml_absent") {
            let key = normalize_toml_path(&param.path)?;
            if normalized_file == "config.toml" && fastctx::is_fastctx_owned_toml_path(&key) {
                return Err(OwnershipError::new(OwnershipCode::ExternalKeyOwned));
            }
            let owners = toml_paths.entry(file_key).or_default();
            let segments = path_parts(&key.replace('.', "/"));
            let absent = param.apply_mode == "toml_absent";
            for (existing, existing_absent) in owners.iter() {
                if existing == &segments {
                    return Err(OwnershipError::new(if absent || *existing_absent {
                        OwnershipCode::TomlAbsentConflict
                    } else {
                        OwnershipCode::DuplicateTomlPath
                    }));
                }
                if is_path_prefix(existing, &segments) || is_path_prefix(&segments, existing) {
                    return Err(OwnershipError::new(if absent || *existing_absent {
                        OwnershipCode::TomlAbsentConflict
                    } else {
                        OwnershipCode::TomlParentChildConflict
                    }));
                }
            }
            owners.push((segments, absent));
        }
    }
    Ok(())
}

/// 写入引擎的最小边界检查；完整的 schema/files 索引由调用方先完成。
pub(crate) fn validate_target_path(
    paths: &AppPaths,
    relative: &str,
) -> Result<String, OwnershipError> {
    let normalized = normalize_relative_path(relative)?;
    if normalized != relative {
        return Err(OwnershipError::new(OwnershipCode::NonCanonicalPath));
    }
    resolve_guard_path(paths, &normalized)?;
    Ok(normalized)
}

fn valid_param_id(id: &str) -> bool {
    !id.is_empty()
        && id.trim() == id
        && !id.chars().any(char::is_whitespace)
        && !id.contains(['\0', '<', '>'])
}

pub(crate) fn normalize_toml_path(path: &str) -> Result<String, OwnershipError> {
    if path.trim().is_empty() {
        return Err(OwnershipError::new(OwnershipCode::InvalidPath));
    }
    let parts = path.split('.').map(str::trim).collect::<Vec<_>>();
    if parts
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == ".." || part.contains(['/', '\\']))
    {
        return Err(OwnershipError::new(OwnershipCode::InvalidPath));
    }
    Ok(parts.join("."))
}

fn resolve_guard_path(paths: &AppPaths, relative: &str) -> Result<PathBuf, OwnershipError> {
    let root = paths.codex_root();
    let root_real = canonicalize_allow_missing(root)?;
    let raw_target = root.join(relative);
    let parts = path_parts(relative);
    let mut cursor = root.to_path_buf();
    for (index, part) in parts.iter().enumerate() {
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(OwnershipError::new(OwnershipCode::PathSymlink));
                }
                if index + 1 < parts.len() && !metadata.is_dir() {
                    return Err(OwnershipError::new(OwnershipCode::PathProbeFailed));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(OwnershipError::new(OwnershipCode::PathProbeFailed)),
        }
    }
    if fs::symlink_metadata(&raw_target)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        return Err(OwnershipError::new(OwnershipCode::PathTargetDirectory));
    }
    let target_real = canonicalize_allow_missing(&raw_target)?;
    if !target_real.starts_with(&root_real) {
        return Err(OwnershipError::new(OwnershipCode::PathOutsideRoot));
    }
    Ok(target_real)
}

fn canonicalize_allow_missing(path: &Path) -> Result<PathBuf, OwnershipError> {
    let mut missing = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) => {
                if !missing.is_empty() && !metadata.is_dir() {
                    return Err(OwnershipError::new(OwnershipCode::PathProbeFailed));
                }
                let mut canonical = fs::canonicalize(&cursor)
                    .map_err(|_| OwnershipError::new(OwnershipCode::PathProbeFailed))?;
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(name) = cursor.file_name() else {
                    return Err(OwnershipError::new(OwnershipCode::PathProbeFailed));
                };
                missing.push(name.to_os_string());
                if !cursor.pop() {
                    return Err(OwnershipError::new(OwnershipCode::PathProbeFailed));
                }
            }
            Err(_) => return Err(OwnershipError::new(OwnershipCode::PathProbeFailed)),
        }
    }
}

fn path_parts(path: &str) -> Vec<String> {
    path.split('/').map(str::to_string).collect()
}

fn path_key(path: &str) -> String {
    #[cfg(windows)]
    {
        path.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

fn casefold(value: &str) -> String {
    value.to_lowercase()
}

fn is_path_prefix(parent: &[String], child: &[String]) -> bool {
    parent.len() < child.len() && child.starts_with(parent)
}

#[cfg(test)]
mod tests {
    use crate::codex_guard::GuardFileFormat;

    use super::*;

    #[test]
    fn normalizes_relative_guard_paths_and_rejects_escape_forms() {
        assert_eq!(
            normalize_relative_path("./agents//default.toml").unwrap(),
            "agents/default.toml"
        );
        assert_eq!(
            normalize_relative_path(r"agents\default.toml").unwrap(),
            "agents/default.toml"
        );
        for invalid in [
            "",
            "../escape",
            "/absolute",
            r"C:\outside",
            r"\\server\share",
            "a\0b",
        ] {
            assert!(
                normalize_relative_path(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_duplicate_file_ids_paths_and_parent_paths() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.codex_root()).unwrap();
        let files = vec![
            GuardFile {
                id: "custom.Config".into(),
                name: "one".into(),
                file: "config.toml".into(),
                format: GuardFileFormat::Toml,
                builtin: false,
                detection: None,
            },
            GuardFile {
                id: "custom.config".into(),
                name: "two".into(),
                file: "./config.toml".into(),
                format: GuardFileFormat::Toml,
                builtin: false,
                detection: None,
            },
        ];
        let error = validate_ownership(&paths, &files, &[]).unwrap_err();
        assert!(matches!(
            error.code,
            OwnershipCode::NonCanonicalPath | OwnershipCode::DuplicateFileId
        ));

        let files = vec![
            GuardFile {
                id: "one".into(),
                name: "one".into(),
                file: "config.toml".into(),
                format: GuardFileFormat::Toml,
                builtin: false,
                detection: None,
            },
            GuardFile {
                id: "two".into(),
                name: "two".into(),
                file: "config.toml".into(),
                format: GuardFileFormat::Toml,
                builtin: false,
                detection: None,
            },
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &[]).unwrap_err().code,
            OwnershipCode::DuplicateFilePath
        );

        let files = vec![
            GuardFile {
                id: "one".into(),
                name: "one".into(),
                file: "agents".into(),
                format: GuardFileFormat::PlainText,
                builtin: false,
                detection: None,
            },
            GuardFile {
                id: "two".into(),
                name: "two".into(),
                file: "agents/default.toml".into(),
                format: GuardFileFormat::Toml,
                builtin: false,
                detection: None,
            },
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &[]).unwrap_err().code,
            OwnershipCode::ParentFilePathConflict
        );
    }

    #[test]
    fn rejects_toml_parent_child_and_file_overwrite_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.codex_root()).unwrap();
        let files = vec![GuardFile {
            id: "config".into(),
            name: "config".into(),
            file: "config.toml".into(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
        }];
        let params = vec![
            param("a", "config.toml", "toml_key", "features", "bool"),
            param("b", "config.toml", "toml_absent", "features.multi", "none"),
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &params)
                .unwrap_err()
                .code,
            OwnershipCode::TomlAbsentConflict
        );

        let params = vec![
            param("a", "config.toml", "toml_key", "features", "bool"),
            param("b", "config.toml", "toml_key", "features.multi", "bool"),
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &params)
                .unwrap_err()
                .code,
            OwnershipCode::TomlParentChildConflict
        );

        let params = vec![
            param("a", "config.toml", "file_overwrite", "", "text"),
            param("b", "config.toml", "toml_key", "features.enabled", "bool"),
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &params)
                .unwrap_err()
                .code,
            OwnershipCode::FileOverwriteConflict
        );
    }

    #[test]
    fn rejects_unknown_files_and_duplicate_parameter_ids() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.codex_root()).unwrap();
        let files = vec![GuardFile {
            id: "config".into(),
            name: "config".into(),
            file: "config.toml".into(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
        }];
        let params = vec![param("a", "missing.toml", "toml_key", "x", "bool")];
        assert_eq!(
            validate_ownership(&paths, &files, &params)
                .unwrap_err()
                .code,
            OwnershipCode::UnknownFile
        );

        let params = vec![
            param("same", "config.toml", "toml_key", "x", "bool"),
            param("SAME", "config.toml", "toml_key", "y", "bool"),
        ];
        assert_eq!(
            validate_ownership(&paths, &files, &params)
                .unwrap_err()
                .code,
            OwnershipCode::DuplicateParamId
        );
    }

    #[test]
    fn rejects_fastctx_owned_paths() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.codex_root()).unwrap();
        let files = vec![GuardFile {
            id: "config".into(),
            name: "config".into(),
            file: "config.toml".into(),
            format: GuardFileFormat::Toml,
            builtin: true,
            detection: None,
        }];
        for path in [
            "mcp_servers.fastctx",
            "mcp_servers.fastctx.command",
            "features.code_mode.direct_only_tool_namespaces",
            "tool_output_token_limit",
        ] {
            let params = vec![param("p", "config.toml", "toml_key", path, "text")];
            assert_eq!(
                validate_ownership(&paths, &files, &params)
                    .unwrap_err()
                    .code,
                OwnershipCode::ExternalKeyOwned
            );
        }
        let params = vec![param(
            "p",
            "config.toml",
            "toml_key",
            "mcp_servers.fastctx_extra",
            "text",
        )];
        assert!(validate_ownership(&paths, &files, &params).is_ok());
    }

    #[test]
    fn builtin_configuration_is_valid_when_codex_root_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let files = crate::codex_guard::files::builtin_files();
        let params: Vec<GuardParam> =
            serde_json::from_str(include_str!("guard_schema.json")).unwrap();
        assert!(validate_ownership(&paths, &files, &params).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_guard_targets() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        std::fs::create_dir_all(paths.codex_root()).unwrap();
        let outside = temp.path().join("outside.toml");
        std::fs::write(&outside, "x = 1\n").unwrap();
        std::os::unix::fs::symlink(&outside, paths.codex_file("linked.toml")).unwrap();
        let files = vec![GuardFile {
            id: "linked".into(),
            name: "linked".into(),
            file: "linked.toml".into(),
            format: GuardFileFormat::Toml,
            builtin: false,
            detection: None,
        }];
        assert_eq!(
            validate_ownership(&paths, &files, &[]).unwrap_err().code,
            OwnershipCode::PathSymlink
        );
    }

    fn param(id: &str, file: &str, mode: &str, path: &str, value_type: &str) -> GuardParam {
        GuardParam {
            id: id.into(),
            label: id.into(),
            label_en: String::new(),
            description: String::new(),
            description_en: String::new(),
            file: file.into(),
            file_id: String::new(),
            group_id: None,
            apply_mode: mode.into(),
            path: path.into(),
            value_type: value_type.into(),
            default: serde_json::Value::Null,
            default_en: serde_json::Value::Null,
            custom: true,
        }
    }
}
