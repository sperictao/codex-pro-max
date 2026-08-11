//! Durable storage boundaries for structured subagent roles.
//!
//! The roles module owns validation and migration decisions. This module owns
//! filesystem/config boundaries: a role id resolves to one direct child of
//! <codex>/agents, role files are replaced atomically, and launcher state
//! stores only the record and desired lifecycle.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;

use super::roles::{
    discover_role, role_file_name, role_relative_path, sort_managed_role_records,
    validate_managed_role_count, validate_role_file_name, validate_role_id,
    validate_unique_role_ids, CapabilitySnapshot, DiscoveredRole, ManagedRole, ManagedRoleRecord,
    ManagedRoleSummary, RoleError, RoleErrorCode, RoleHealth, RoleId, RoleLifecycle,
    RoleMigrationChoice, RoleMigrationPlan, RoleMigrationResult,
};
use super::AppPaths;

/// Persisted role state. The TOML document is the source of truth for all
/// editable fields; health and actual file presence are derived at read time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRoleState {
    pub record: ManagedRoleRecord,
    pub lifecycle: RoleLifecycle,
}

impl ManagedRoleState {
    pub fn new(record: ManagedRoleRecord) -> Self {
        Self {
            record,
            lifecycle: RoleLifecycle::Disabled,
        }
    }

    /// 按请求顺序重排角色状态。record 与 lifecycle 必须整体移动：先重排 record
    /// 再按位置拼回 lifecycle 会把生命周期错位到别的角色。
    pub fn reorder(
        states: &mut Vec<ManagedRoleState>,
        requested: &[RoleId],
    ) -> Result<(), RoleError> {
        let mut records = states
            .iter()
            .map(|managed| managed.record.clone())
            .collect::<Vec<_>>();
        super::roles::reorder_role_records(&mut records, requested)?;
        let mut lifecycles = states
            .iter()
            .map(|managed| (managed.record.id.clone(), managed.lifecycle))
            .collect::<std::collections::HashMap<_, _>>();
        let mut reordered = Vec::with_capacity(records.len());
        for record in records {
            let lifecycle = lifecycles
                .remove(&record.id)
                .ok_or_else(|| RoleError::new(RoleErrorCode::InvalidRoleOrder))?;
            reordered.push(ManagedRoleState { record, lifecycle });
        }
        *states = reordered;
        Ok(())
    }

    pub fn id(&self) -> &RoleId {
        &self.record.id
    }

    pub fn validate(&self) -> Result<(), RoleError> {
        self.record.validate(None)
    }

    pub fn into_managed(
        self,
        actual_file_present: bool,
        capabilities: Option<&CapabilitySnapshot>,
    ) -> Result<ManagedRole, RoleError> {
        let health = derive_role_health(&self.record, actual_file_present, capabilities);
        ManagedRole::from_record(self.record, self.lifecycle, health, actual_file_present)
    }
}

pub fn validate_role_states(states: &[ManagedRoleState]) -> Result<(), RoleError> {
    validate_managed_role_count(states.len())?;
    validate_unique_role_ids(
        &states
            .iter()
            .map(|state| state.record.id.clone())
            .collect::<Vec<_>>(),
    )?;
    for state in states {
        state.validate()?;
    }
    Ok(())
}

pub(crate) fn load_role_states(store: &ConfigStore) -> Result<Vec<ManagedRoleState>, String> {
    let states = store
        .load_launcher()?
        .codex_guard
        .roles
        .into_iter()
        .collect::<Vec<_>>();
    validate_role_states(&states).map_err(role_error_code)?;
    Ok(states)
}

fn role_error_code(error: RoleError) -> String {
    error.to_string()
}

pub(crate) fn role_target(paths: &AppPaths, id: &RoleId) -> Result<PathBuf, String> {
    validate_role_id(id.as_str())
        .map_err(role_error_code)
        .map(|_| paths.codex_file(role_relative_path(id)))
}

fn agents_root(paths: &AppPaths) -> PathBuf {
    paths.codex_file(super::roles::ROLE_DIRECTORY)
}

fn ensure_safe_agents_root(paths: &AppPaths, create: bool) -> Result<PathBuf, String> {
    let root = agents_root(paths);
    match fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err("role_directory_invalid".to_string())
        }
        Ok(_) => Ok(root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            fs::create_dir_all(&root).map_err(|_| "role_directory_create_failed".to_string())?;
            let metadata =
                fs::symlink_metadata(&root).map_err(|_| "role_directory_invalid".to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("role_directory_invalid".to_string());
            }
            Ok(root)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(root),
        Err(_) => Err("role_directory_read_failed".to_string()),
    }
}

fn ensure_safe_role_target(
    paths: &AppPaths,
    id: &RoleId,
    create_parent: bool,
) -> Result<PathBuf, String> {
    let root = ensure_safe_agents_root(paths, create_parent)?;
    let file_name = role_file_name(id);
    validate_role_file_name(id, &file_name).map_err(role_error_code)?;
    let target = root.join(file_name);
    match fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("role_file_invalid".to_string())
        }
        Ok(_) => Ok(target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(_) => Err("role_file_stat_failed".to_string()),
    }
}

pub(crate) fn read_role_file(paths: &AppPaths, id: &RoleId) -> Result<Option<Vec<u8>>, String> {
    let target = ensure_safe_role_target(paths, id, false)?;
    match fs::read(&target) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("role_file_read_failed".to_string()),
    }
}

pub(crate) fn role_file_present(paths: &AppPaths, id: &RoleId) -> Result<bool, String> {
    // Presence must not read the whole file: role summaries call this once per role on every
    // page load, and the bytes are discarded immediately.
    let target = ensure_safe_role_target(paths, id, false)?;
    match fs::symlink_metadata(&target) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("role_file_stat_failed".to_string()),
    }
}

/// Discover only direct agents/*.toml children. Nested files and symlink
/// entries are ignored rather than followed.
pub(crate) fn discover_role_files(
    paths: &AppPaths,
    managed: &[ManagedRoleState],
    capabilities: Option<&CapabilitySnapshot>,
) -> Result<Vec<DiscoveredRole>, String> {
    let root = ensure_safe_agents_root(paths, false)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let managed_ids = managed
        .iter()
        .map(|state| state.record.id.clone())
        .collect::<Vec<_>>();
    let mut discovered = Vec::new();
    for entry in fs::read_dir(&root).map_err(|_| "role_directory_read_failed".to_string())? {
        let entry = entry.map_err(|_| "role_directory_read_failed".to_string())?;
        let path = entry.path();
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| "role_file_stat_failed".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.ends_with(super::roles::ROLE_EXTENSION) {
            continue;
        }
        let relative = format!("{}/{}", super::roles::ROLE_DIRECTORY, file_name);
        let bytes = fs::read(&path).map_err(|_| "role_file_read_failed".to_string())?;
        let text = std::str::from_utf8(&bytes).map_err(|_| "role_file_read_failed".to_string())?;
        discovered.push(discover_role(&relative, text, &managed_ids, capabilities));
    }
    discovered.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.relative_file_name.cmp(&right.relative_file_name))
    });
    Ok(discovered)
}

pub(crate) fn role_summaries(
    paths: &AppPaths,
    states: &[ManagedRoleState],
    capabilities: Option<&CapabilitySnapshot>,
) -> Result<Vec<ManagedRoleSummary>, String> {
    validate_role_states(states).map_err(role_error_code)?;
    let mut states = states.to_vec();
    states.sort_by(|left, right| {
        left.record
            .id
            .is_default()
            .cmp(&right.record.id.is_default())
            .reverse()
            .then_with(|| left.record.order.cmp(&right.record.order))
            .then_with(|| left.record.id.cmp(&right.record.id))
    });
    states
        .into_iter()
        .map(|state| {
            let present = role_file_present(paths, &state.record.id)?;
            state
                .into_managed(present, capabilities)
                .map(|managed| managed.summary())
                .map_err(role_error_code)
        })
        .collect()
}

fn derive_role_health(
    record: &ManagedRoleRecord,
    actual_file_present: bool,
    capabilities: Option<&CapabilitySnapshot>,
) -> RoleHealth {
    let view = match record.view() {
        Ok(view) => view,
        Err(_) => return RoleHealth::Invalid,
    };
    if let Some(snapshot) = capabilities {
        if let Err(error) = super::roles::validate_role_capability(&view, snapshot) {
            return match error.code {
                RoleErrorCode::UnsupportedModel
                | RoleErrorCode::UnsupportedReasoningEffort
                | RoleErrorCode::UnsupportedMultiAgentVersion => RoleHealth::Unsupported,
                _ => RoleHealth::Invalid,
            };
        }
    } else {
        return RoleHealth::Error;
    }
    if actual_file_present {
        RoleHealth::Healthy
    } else {
        RoleHealth::Drifted
    }
}

pub const ROLE_DIRECTORY_BEGIN: &str = "<!-- dashi:begin subagent-role-directory -->";
pub const ROLE_DIRECTORY_END: &str = "<!-- dashi:end subagent-role-directory -->";
pub const ROLE_DIRECTORY_FILE: &str = "AGENTS.md";
pub const ROLE_DIRECTORY_MEMBER_ID: &str = "subagent-role-directory";

pub(crate) fn render_role_directory(
    states: &[ManagedRoleState],
    present_ids: &HashSet<String>,
) -> Result<String, String> {
    validate_role_states(states).map_err(role_error_code)?;
    let mut records = states
        .iter()
        .filter(|state| present_ids.contains(state.record.id.as_str()))
        .map(|state| state.record.clone())
        .collect::<Vec<_>>();
    sort_managed_role_records(&mut records);
    let mut lines = vec![ROLE_DIRECTORY_BEGIN.to_string()];
    lines.push(
        "Use the exact agent_type below and always set fork_turns=\"none\"; omit spawn model and reasoning overrides. Role files provide model and effort.".to_string(),
    );
    for record in records {
        let view = record.view().map_err(role_error_code)?;
        lines.push(format!(
            "- agent_type={} - {}: {}; choose when {}.",
            record.id,
            markdown_single_line(&view.display_name),
            markdown_single_line(&view.purpose),
            markdown_single_line(&record.selection_criteria),
        ));
    }
    lines.push(ROLE_DIRECTORY_END.to_string());
    Ok(lines.join("\n"))
}

/// Return only the managed block body so the shared Guard planner can merge it
/// with other `AGENTS.md` blocks in one physical-file write.
pub(crate) fn render_role_directory_content(
    states: &[ManagedRoleState],
    present_ids: &HashSet<String>,
) -> Result<String, String> {
    let block = render_role_directory(states, present_ids)?;
    let mut lines = block.lines();
    let begin = lines.next();
    let end = lines.clone().last().map(str::to_string).unwrap_or_default();
    if begin != Some(ROLE_DIRECTORY_BEGIN) || end != ROLE_DIRECTORY_END {
        return Err("role_directory_render_invalid".to_string());
    }
    let body = lines.collect::<Vec<_>>();
    if body.is_empty() {
        return Err("role_directory_render_invalid".to_string());
    }
    Ok(body[..body.len() - 1].join("\n"))
}

fn markdown_single_line(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect::<String>()
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('|', "\\|")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn replace_role_directory_block(
    document: &str,
    states: &[ManagedRoleState],
    present_ids: &HashSet<String>,
) -> Result<String, String> {
    let block = render_role_directory(states, present_ids)?;
    let begin_count = document.matches(ROLE_DIRECTORY_BEGIN).count();
    let end_count = document.matches(ROLE_DIRECTORY_END).count();
    if begin_count > 1 || end_count > 1 || begin_count != end_count {
        return Err("role_directory_markers_invalid".to_string());
    }
    if begin_count == 0 {
        let separator = if document.is_empty() || document.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return Ok(format!("{document}{separator}\n{block}\n"));
    }
    let begin = document
        .find(ROLE_DIRECTORY_BEGIN)
        .ok_or_else(|| "role_directory_markers_invalid".to_string())?;
    let end_start = document
        .find(ROLE_DIRECTORY_END)
        .ok_or_else(|| "role_directory_markers_invalid".to_string())?;
    if end_start < begin {
        return Err("role_directory_markers_invalid".to_string());
    }
    let end = end_start + ROLE_DIRECTORY_END.len();
    let mut result = String::with_capacity(document.len() + block.len());
    result.push_str(&document[..begin]);
    result.push_str(&block);
    result.push_str(&document[end..]);
    Ok(result)
}

pub(crate) fn plan_default_migration(
    stored_expected: Option<&str>,
    live_toml: Option<&str>,
    previous_lifecycle: RoleLifecycle,
    safe_default_toml: &str,
) -> Result<RoleMigrationPlan, String> {
    super::roles::plan_default_role_migration(
        stored_expected,
        live_toml,
        previous_lifecycle,
        safe_default_toml,
    )
    .map_err(role_error_code)
}

pub(crate) fn resolve_default_migration(
    choice: RoleMigrationChoice,
    stored_expected: Option<&str>,
    live_toml: Option<&str>,
    safe_default_toml: &str,
    previous_lifecycle: RoleLifecycle,
    now_ms: u64,
) -> Result<RoleMigrationResult, String> {
    super::roles::resolve_default_role_migration(
        choice,
        stored_expected,
        live_toml,
        safe_default_toml,
        previous_lifecycle,
        now_ms,
    )
    .map_err(role_error_code)
}

pub(crate) fn content_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_guard::atomic_store::{AtomicFileWriter, PlatformAtomicFileWriter};
    use crate::codex_guard::roles::{
        build_managed_role_record, ModelCapability, RoleEditableFields, TextOrigin,
    };

    fn capabilities() -> CapabilitySnapshot {
        CapabilitySnapshot::new(
            vec![ModelCapability::new("model-v2", "v2", vec!["low".into()]).unwrap()],
            "fixture",
            1,
        )
        .unwrap()
    }

    fn fields() -> RoleEditableFields {
        RoleEditableFields {
            display_name: "Reviewer".into(),
            purpose: "Review changes".into(),
            selection_criteria: "review code".into(),
            model: "model-v2".into(),
            effort: "low".into(),
            instructions: "Review the change.".into(),
        }
    }

    fn states() -> Vec<ManagedRoleState> {
        vec![ManagedRoleState::new(
            build_managed_role_record(
                RoleId::new("reviewer").unwrap(),
                &fields(),
                1,
                1,
                &capabilities(),
                TextOrigin::User,
                TextOrigin::User,
            )
            .unwrap(),
        )]
    }

    #[test]
    fn role_directory_is_single_line_and_does_not_include_instructions() {
        let mut present = HashSet::new();
        present.insert("reviewer".to_string());
        let block = render_role_directory(&states(), &present).unwrap();
        assert!(block.contains("agent_type"));
        assert!(block.contains("fork_turns=\"none\""));
        assert!(block.contains("Review changes"));
        assert!(!block.contains("Review the change."));
        assert!(block.lines().all(|line| !line.contains('\r')));
    }

    /// Reordering must move `record` and `lifecycle` together. Rebuilding the record
    /// list and zipping it back positionally silently hands a Locked role's lifecycle
    /// to whichever role took its old slot.
    #[test]
    fn reorder_keeps_each_lifecycle_with_its_own_role() {
        let make = |id: &str, lifecycle: RoleLifecycle| ManagedRoleState {
            record: build_managed_role_record(
                RoleId::new(id).unwrap(),
                &fields(),
                1,
                1,
                &capabilities(),
                TextOrigin::User,
                TextOrigin::User,
            )
            .unwrap(),
            lifecycle,
        };
        let mut states = vec![
            make("default", RoleLifecycle::Locked),
            make("reviewer", RoleLifecycle::Disabled),
            make("writer", RoleLifecycle::Locked),
        ];
        let requested = vec![
            RoleId::new("default").unwrap(),
            RoleId::new("writer").unwrap(),
            RoleId::new("reviewer").unwrap(),
        ];

        ManagedRoleState::reorder(&mut states, &requested).unwrap();

        assert_eq!(
            states
                .iter()
                .map(|managed| (managed.id().as_str(), managed.lifecycle))
                .collect::<Vec<_>>(),
            vec![
                ("default", RoleLifecycle::Locked),
                ("writer", RoleLifecycle::Locked),
                ("reviewer", RoleLifecycle::Disabled),
            ],
            "reorder must not transplant a lifecycle onto a different role"
        );
        assert_eq!(
            states
                .iter()
                .map(|managed| managed.record.order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn role_directory_replacement_preserves_surrounding_document() {
        let mut present = HashSet::new();
        present.insert("reviewer".to_string());
        let old = "before\n<!-- dashi:begin subagent-role-directory -->\nold\n<!-- dashi:end subagent-role-directory -->\nafter\n";
        let updated = replace_role_directory_block(old, &states(), &present).unwrap();
        assert!(updated.starts_with("before\n"));
        assert!(updated.ends_with("after\n"));
        assert!(!updated.contains("\nold\n"));
    }

    #[test]
    fn role_directory_content_is_ready_for_shared_markdown_planning() {
        let mut present = HashSet::new();
        present.insert("reviewer".to_string());
        let content = render_role_directory_content(&states(), &present).unwrap();
        assert!(!content.contains(ROLE_DIRECTORY_BEGIN));
        assert!(!content.contains(ROLE_DIRECTORY_END));
        assert!(content.contains("Use the exact agent_type below"));
        let document = format!(
            "before\n{}\n{}\n{}\nafter\n",
            ROLE_DIRECTORY_BEGIN, content, ROLE_DIRECTORY_END
        );
        let replaced = replace_role_directory_block(&document, &states(), &present).unwrap();
        assert!(replaced.contains(content.trim()));
        assert!(replaced.starts_with("before\n"));
        assert!(replaced.ends_with("after\n"));
    }

    #[test]
    fn role_directory_rejects_reversed_markers() {
        let mut present = HashSet::new();
        present.insert("reviewer".to_string());
        let reversed = format!(
            "before\n{}\nold\n{}\nafter\n",
            ROLE_DIRECTORY_END, ROLE_DIRECTORY_BEGIN
        );
        assert_eq!(
            replace_role_directory_block(&reversed, &states(), &present).unwrap_err(),
            "role_directory_markers_invalid"
        );
    }

    #[test]
    fn role_file_write_is_atomic_and_rejects_symlink_target() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let id = RoleId::new("reviewer").unwrap();
        let target = ensure_safe_role_target(&paths, &id, true).unwrap();
        PlatformAtomicFileWriter
            .replace(&target, b"name = \"Reviewer\"\n")
            .unwrap();
        assert_eq!(
            read_role_file(&paths, &id).unwrap().unwrap(),
            b"name = \"Reviewer\"\n"
        );
        #[cfg(unix)]
        {
            let outside = temp.path().join("outside.toml");
            fs::write(&outside, b"outside").unwrap();
            fs::remove_file(&target).unwrap();
            std::os::unix::fs::symlink(&outside, &target).unwrap();
            assert_eq!(
                ensure_safe_role_target(&paths, &id, true).unwrap_err(),
                "role_file_invalid"
            );
            assert_eq!(fs::read(&outside).unwrap(), b"outside");
        }
    }

    #[test]
    fn discovery_ignores_nested_and_symlink_files() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temp.path());
        let root = paths.codex_file("agents");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("reviewer.toml"), role_toml()).unwrap();
        fs::write(root.join("nested/ignored.toml"), role_toml()).unwrap();
        #[cfg(unix)]
        {
            let outside = temp.path().join("outside.toml");
            fs::write(&outside, role_toml()).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("linked.toml")).unwrap();
        }
        let discovered = discover_role_files(&paths, &[], Some(&capabilities())).unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id, "reviewer");
    }

    fn role_toml() -> String {
        "name = \"Reviewer\"\ndescription = \"Review changes\"\nmodel = \"model-v2\"\nmodel_reasoning_effort = \"low\"\ndeveloper_instructions = \"Review the change.\"\n".to_string()
    }
}
