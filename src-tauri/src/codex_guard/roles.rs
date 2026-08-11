//! Pure domain logic for structured Codex subagent roles.
//!
//! This module deliberately does not read the filesystem, invoke Codex, or expose
//! role instructions in discovery summaries.  The command and transaction layers
//! can use these value objects at their boundaries.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use toml_edit::{DocumentMut, Item, Value};

pub const DEFAULT_ROLE_ID: &str = "default";
pub const MAX_MANAGED_ROLES: usize = 32;
#[cfg(test)]
pub const ROLE_LIMIT_WARNING_THRESHOLD: usize = 28;
pub const ROLE_ID_MAX_BYTES: usize = 63;
pub const DISPLAY_NAME_MAX_CHARS: usize = 80;
pub const PURPOSE_MAX_CHARS: usize = 500;
pub const SELECTION_CRITERIA_MAX_CHARS: usize = 500;
pub const INSTRUCTIONS_MAX_BYTES: usize = 64 * 1024;
pub const ROLE_DIRECTORY: &str = "agents";
pub const ROLE_EXTENSION: &str = ".toml";
pub const DEFAULT_SELECTION_CRITERIA: &str = "default role";

pub const KNOWN_ROLE_KEYS: [&str; 5] = [
    "name",
    "description",
    "model",
    "model_reasoning_effort",
    "developer_instructions",
];

/// Stable, localisable error identity.  It intentionally carries no input value,
/// path, parser output, or role instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleErrorCode {
    InvalidRoleId,
    InvalidRoleFileName,
    DuplicateRoleId,
    DefaultRoleProtected,
    TooManyManagedRoles,
    InvalidText,
    TextTooLong,
    MissingRoleField,
    InvalidRoleFieldType,
    UnknownRoleField,
    InvalidRoleToml,
    InvalidRoleOrder,
    InvalidRoleLifecycle,
    UnsupportedModel,
    UnsupportedReasoningEffort,
    UnsupportedMultiAgentVersion,
    CapabilityUnavailable,
    MigrationSourceUnavailable,
    MigrationSourceInvalid,
    SafeDefaultInvalid,
    InvalidMigrationChoice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleError {
    pub code: RoleErrorCode,
    /// A known schema field name only; never a user supplied value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
}

impl RoleError {
    pub const fn new(code: RoleErrorCode) -> Self {
        Self { code, field: None }
    }

    fn field(code: RoleErrorCode, field: &'static str) -> Self {
        Self {
            code,
            field: Some(field.to_string()),
        }
    }
}

impl fmt::Display for RoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.code {
            RoleErrorCode::InvalidRoleId => "role.invalid_id",
            RoleErrorCode::InvalidRoleFileName => "role.invalid_file_name",
            RoleErrorCode::DuplicateRoleId => "role.duplicate_id",
            RoleErrorCode::DefaultRoleProtected => "role.default_protected",
            RoleErrorCode::TooManyManagedRoles => "role.too_many_managed",
            RoleErrorCode::InvalidText => "role.invalid_text",
            RoleErrorCode::TextTooLong => "role.text_too_long",
            RoleErrorCode::MissingRoleField => "role.missing_field",
            RoleErrorCode::InvalidRoleFieldType => "role.invalid_field_type",
            RoleErrorCode::UnknownRoleField => "role.unknown_field",
            RoleErrorCode::InvalidRoleToml => "role.invalid_toml",
            RoleErrorCode::InvalidRoleOrder => "role.invalid_order",
            RoleErrorCode::InvalidRoleLifecycle => "role.invalid_lifecycle",
            RoleErrorCode::UnsupportedModel => "role.unsupported_model",
            RoleErrorCode::UnsupportedReasoningEffort => "role.unsupported_reasoning_effort",
            RoleErrorCode::UnsupportedMultiAgentVersion => "role.unsupported_multi_agent_version",
            RoleErrorCode::CapabilityUnavailable => "role.capability_unavailable",
            RoleErrorCode::MigrationSourceUnavailable => "role.migration_source_unavailable",
            RoleErrorCode::MigrationSourceInvalid => "role.migration_source_invalid",
            RoleErrorCode::SafeDefaultInvalid => "role.safe_default_invalid",
            RoleErrorCode::InvalidMigrationChoice => "role.invalid_migration_choice",
        })
    }
}

impl std::error::Error for RoleError {}

/// Role IDs are the Codex `agent_type` and the filename stem.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RoleId(String);

impl RoleId {
    pub fn new(raw: &str) -> Result<Self, RoleError> {
        if raw.is_empty() || raw.len() > ROLE_ID_MAX_BYTES || raw.trim() != raw {
            return Err(RoleError::new(RoleErrorCode::InvalidRoleId));
        }
        if !raw.is_ascii()
            || raw
                .as_bytes()
                .first()
                .is_none_or(|b| !b.is_ascii_lowercase())
        {
            return Err(RoleError::new(RoleErrorCode::InvalidRoleId));
        }
        if !raw
            .as_bytes()
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        {
            return Err(RoleError::new(RoleErrorCode::InvalidRoleId));
        }
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }

    pub fn is_default(&self) -> bool {
        self.as_str() == DEFAULT_ROLE_ID
    }
}

pub fn validate_role_id(raw: &str) -> Result<RoleId, RoleError> {
    RoleId::new(raw)
}

#[cfg(test)]
#[allow(dead_code)]
pub fn validate_new_role_id(raw: &str, existing: &[RoleId]) -> Result<RoleId, RoleError> {
    let id = RoleId::new(raw)?;
    if id.is_default() {
        return Err(RoleError::new(RoleErrorCode::DefaultRoleProtected));
    }
    ensure_role_id_available(&id, existing)?;
    Ok(id)
}

impl fmt::Debug for RoleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RoleId").field(&self.0).finish()
    }
}

impl fmt::Display for RoleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for RoleId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<&str> for RoleId {
    type Error = RoleError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for RoleId {
    type Error = RoleError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<RoleId> for String {
    fn from(value: RoleId) -> Self {
        value.0
    }
}

impl Serialize for RoleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RoleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::new(&raw).map_err(serde::de::Error::custom)
    }
}

pub fn role_file_name(id: &RoleId) -> String {
    format!("{}{}", id.as_str(), ROLE_EXTENSION)
}

pub fn role_relative_path(id: &RoleId) -> String {
    format!("{}/{}", ROLE_DIRECTORY, role_file_name(id))
}

/// Validate a filename that is expected to be directly under `agents/`.
pub fn validate_role_file_name(id: &RoleId, file_name: &str) -> Result<(), RoleError> {
    if file_name.is_empty()
        || file_name.contains('\0')
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name != role_file_name(id)
    {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleFileName));
    }
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
pub fn validate_role_filename(id: &RoleId, file_name: &str) -> Result<(), RoleError> {
    validate_role_file_name(id, file_name)
}

/// Validate a relative Codex path, including the required `agents/` directory.
#[cfg(test)]
#[allow(dead_code)]
pub fn validate_role_relative_path(id: &RoleId, relative_path: &str) -> Result<(), RoleError> {
    if relative_path != role_relative_path(id) || relative_path.contains('\\') {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleFileName));
    }
    Ok(())
}

pub fn role_id_from_file_name(file_name: &str) -> Result<RoleId, RoleError> {
    if file_name.contains('/') || file_name.contains('\\') || !file_name.ends_with(ROLE_EXTENSION) {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleFileName));
    }
    RoleId::new(&file_name[..file_name.len() - ROLE_EXTENSION.len()])
        .map_err(|_| RoleError::new(RoleErrorCode::InvalidRoleFileName))
}

fn role_id_key(id: &RoleId) -> String {
    id.as_str().to_ascii_lowercase()
}

pub fn validate_unique_role_ids(ids: &[RoleId]) -> Result<(), RoleError> {
    let mut seen = HashSet::with_capacity(ids.len());
    for id in ids {
        if !seen.insert(role_id_key(id)) {
            return Err(RoleError::new(RoleErrorCode::DuplicateRoleId));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
pub fn ensure_role_id_available(candidate: &RoleId, existing: &[RoleId]) -> Result<(), RoleError> {
    if existing
        .iter()
        .any(|existing_id| role_id_key(existing_id) == role_id_key(candidate))
    {
        Err(RoleError::new(RoleErrorCode::DuplicateRoleId))
    } else {
        Ok(())
    }
}

pub fn validate_managed_role_count(count: usize) -> Result<(), RoleError> {
    if count > MAX_MANAGED_ROLES {
        Err(RoleError::new(RoleErrorCode::TooManyManagedRoles))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub fn validate_role_count(count: usize) -> Result<(), RoleError> {
    validate_managed_role_count(count)
}

#[cfg(test)]
#[allow(dead_code)]
pub fn role_count_near_limit(count: usize) -> bool {
    (ROLE_LIMIT_WARNING_THRESHOLD..=MAX_MANAGED_ROLES).contains(&count)
}

#[cfg(test)]
#[allow(dead_code)]
pub fn can_add_managed_role(count: usize) -> Result<(), RoleError> {
    validate_managed_role_count(count.saturating_add(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextOrigin {
    Builtin,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleLifecycle {
    Disabled,
    Applied,
    Locked,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleLifecycleAction {
    Apply,
    Lock,
    Unlock,
    Disable,
}

impl RoleLifecycle {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn transition(self, action: RoleLifecycleAction) -> Result<Self, RoleError> {
        Ok(match action {
            RoleLifecycleAction::Apply => match self {
                Self::Disabled | Self::Applied => Self::Applied,
                Self::Locked => Self::Locked,
            },
            RoleLifecycleAction::Lock => Self::Locked,
            RoleLifecycleAction::Unlock => match self {
                Self::Locked => Self::Applied,
                Self::Disabled => Self::Disabled,
                Self::Applied => Self::Applied,
            },
            RoleLifecycleAction::Disable => Self::Disabled,
        })
    }

    pub fn from_flags(applied: bool, locked: bool) -> Result<Self, RoleError> {
        match (applied, locked) {
            (false, false) => Ok(Self::Disabled),
            (true, false) => Ok(Self::Applied),
            (true, true) => Ok(Self::Locked),
            (false, true) => Err(RoleError::new(RoleErrorCode::InvalidRoleLifecycle)),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Locked)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleHealth {
    Healthy,
    Drifted,
    Invalid,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleAction {
    Rename,
    CopyInto,
    StopManaging,
    Delete,
}

pub fn validate_default_action(id: &RoleId, action: RoleAction) -> Result<(), RoleError> {
    if id.is_default()
        && matches!(
            action,
            RoleAction::CopyInto
                | RoleAction::StopManaging
                | RoleAction::Delete
                | RoleAction::Rename
        )
    {
        Err(RoleError::new(RoleErrorCode::DefaultRoleProtected))
    } else {
        Ok(())
    }
}

pub fn validate_copy_roles(source: &RoleId, target: &RoleId) -> Result<(), RoleError> {
    if source == target {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleId));
    }
    // The protected check applies to the destination.  Copying content from
    // default into a new role is allowed; overwriting default is not.
    validate_default_action(target, RoleAction::CopyInto)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRoleRecord {
    pub id: RoleId,
    pub selection_criteria: String,
    pub order: u32,
    pub expected_toml: String,
    pub policy_revision: u64,
    pub policy_hash: String,
    pub policy_updated_at_ms: u64,
    pub description_origin: TextOrigin,
    pub instructions_origin: TextOrigin,
}

impl ManagedRoleRecord {
    pub fn view(&self) -> Result<ManagedRoleView, RoleError> {
        Ok(parse_role_toml(&self.expected_toml)?.view)
    }

    pub fn validate(&self, capabilities: Option<&CapabilitySnapshot>) -> Result<(), RoleError> {
        validate_selection_criteria(&self.selection_criteria)?;
        let view = self.view()?;
        if let Some(snapshot) = capabilities {
            validate_role_capability(&view, snapshot)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRole {
    pub record: ManagedRoleRecord,
    pub lifecycle: RoleLifecycle,
    pub health: RoleHealth,
    pub view: ManagedRoleView,
    /// Whether `agents/<id>.toml` currently exists.  This is a summary fact and
    /// does not carry the absolute path or document bytes.
    pub actual_file_present: bool,
}

impl ManagedRole {
    pub fn from_record(
        record: ManagedRoleRecord,
        lifecycle: RoleLifecycle,
        health: RoleHealth,
        actual_file_present: bool,
    ) -> Result<Self, RoleError> {
        let view = record.view()?;
        Ok(Self {
            record,
            lifecycle,
            health,
            view,
            actual_file_present,
        })
    }

    pub fn summary(&self) -> ManagedRoleSummary {
        ManagedRoleSummary {
            id: self.record.id.clone(),
            display_name: self.view.display_name.clone(),
            purpose: self.view.purpose.clone(),
            selection_criteria: self.record.selection_criteria.clone(),
            model: self.view.model.clone(),
            effort: self.view.effort.clone(),
            order: self.record.order,
            lifecycle: self.lifecycle,
            health: self.health,
            actual_file_present: self.actual_file_present,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRoleSummary {
    pub id: RoleId,
    pub display_name: String,
    pub purpose: String,
    pub selection_criteria: String,
    pub model: String,
    pub effort: String,
    pub order: u32,
    pub lifecycle: RoleLifecycle,
    pub health: RoleHealth,
    pub actual_file_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleEditableFields {
    pub display_name: String,
    pub purpose: String,
    pub selection_criteria: String,
    pub model: String,
    pub effort: String,
    /// Instructions are intentionally omitted from discovered summaries.  They
    /// are only present in this write-side draft or a detail view.
    pub instructions: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRoleView {
    pub display_name: String,
    pub purpose: String,
    pub model: String,
    pub effort: String,
    pub instructions: String,
    pub warnings: Vec<RoleWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleWarningCode {
    UnknownField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleWarning {
    pub code: RoleWarningCode,
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRoleToml {
    pub view: ManagedRoleView,
    pub warnings: Vec<RoleWarning>,
}

pub fn parse_role_toml(text: &str) -> Result<ParsedRoleToml, RoleError> {
    if text.contains('\0') {
        return Err(RoleError::new(RoleErrorCode::InvalidText));
    }
    let document = text
        .parse::<DocumentMut>()
        .map_err(|_| RoleError::new(RoleErrorCode::InvalidRoleToml))?;
    let display_name = required_string(&document, "name")?;
    let purpose = required_string(&document, "description")?;
    let model = required_string(&document, "model")?;
    let effort = required_string(&document, "model_reasoning_effort")?;
    let instructions = required_string(&document, "developer_instructions")?;
    validate_display_name(&display_name)?;
    validate_purpose(&purpose)?;
    validate_model(&model)?;
    validate_effort(&effort)?;
    validate_instructions(&instructions)?;

    let mut warnings = Vec::new();
    for (key, _) in document.iter() {
        if !KNOWN_ROLE_KEYS.contains(&key) {
            warnings.push(RoleWarning {
                code: RoleWarningCode::UnknownField,
                field: safe_field_name(key),
            });
        }
    }
    warnings.sort_by(|left, right| left.field.cmp(&right.field));
    Ok(ParsedRoleToml {
        view: ManagedRoleView {
            display_name,
            purpose,
            model,
            effort,
            instructions,
            warnings: warnings.clone(),
        },
        warnings,
    })
}

fn safe_field_name(field: &str) -> Option<String> {
    if field.is_empty()
        || field.len() > 64
        || !field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        None
    } else {
        Some(field.to_string())
    }
}

#[allow(dead_code)]
pub fn parse_role_toml_bytes(bytes: &[u8]) -> Result<ParsedRoleToml, RoleError> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| RoleError::new(RoleErrorCode::InvalidText))?;
    parse_role_toml(text)
}

fn required_string(document: &DocumentMut, field: &'static str) -> Result<String, RoleError> {
    let value = document
        .get(field)
        .ok_or_else(|| RoleError::field(RoleErrorCode::MissingRoleField, field))?
        .as_str()
        .ok_or_else(|| RoleError::field(RoleErrorCode::InvalidRoleFieldType, field))?;
    if value.contains('\0') {
        return Err(RoleError::field(RoleErrorCode::InvalidText, field));
    }
    Ok(value.trim().to_string())
}

fn validate_display_name(value: &str) -> Result<(), RoleError> {
    validate_bounded_text(value, 1, DISPLAY_NAME_MAX_CHARS, false)
}

fn validate_purpose(value: &str) -> Result<(), RoleError> {
    validate_bounded_text(value, 1, PURPOSE_MAX_CHARS, true)
}

fn validate_selection_criteria(value: &str) -> Result<(), RoleError> {
    validate_bounded_text(value, 1, SELECTION_CRITERIA_MAX_CHARS, true)
}

fn validate_model(value: &str) -> Result<(), RoleError> {
    validate_bounded_text(value, 1, 256, false)
}

fn validate_effort(value: &str) -> Result<(), RoleError> {
    validate_bounded_text(value, 1, 64, false)
}

fn validate_instructions(value: &str) -> Result<(), RoleError> {
    if value.contains('\0') || value.trim().is_empty() {
        return Err(RoleError::new(RoleErrorCode::InvalidText));
    }
    if value.len() > INSTRUCTIONS_MAX_BYTES {
        return Err(RoleError::new(RoleErrorCode::TextTooLong));
    }
    Ok(())
}

fn validate_bounded_text(
    value: &str,
    min_chars: usize,
    max_chars: usize,
    allow_newline: bool,
) -> Result<(), RoleError> {
    if value.contains('\0') || value.trim().is_empty() {
        return Err(RoleError::new(RoleErrorCode::InvalidText));
    }
    if !allow_newline && value.chars().any(|ch| ch == '\n' || ch == '\r') {
        return Err(RoleError::new(RoleErrorCode::InvalidText));
    }
    let length = value.chars().count();
    if length < min_chars || length > max_chars {
        return Err(RoleError::new(RoleErrorCode::TextTooLong));
    }
    Ok(())
}

pub fn validate_editable_fields(
    fields: &RoleEditableFields,
    capabilities: &CapabilitySnapshot,
) -> Result<(), RoleError> {
    validate_display_name(fields.display_name.trim())?;
    validate_purpose(fields.purpose.trim())?;
    validate_selection_criteria(fields.selection_criteria.trim())?;
    validate_model(fields.model.trim())?;
    validate_effort(fields.effort.trim())?;
    validate_instructions(&fields.instructions)?;
    validate_role_capability_values(&fields.model, &fields.effort, capabilities)
}

pub fn update_role_toml(text: &str, fields: &RoleEditableFields) -> Result<String, RoleError> {
    if text.contains('\0') {
        return Err(RoleError::new(RoleErrorCode::InvalidText));
    }
    validate_display_name(fields.display_name.trim())?;
    validate_purpose(fields.purpose.trim())?;
    validate_selection_criteria(fields.selection_criteria.trim())?;
    validate_model(fields.model.trim())?;
    validate_effort(fields.effort.trim())?;
    validate_instructions(&fields.instructions)?;
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|_| RoleError::new(RoleErrorCode::InvalidRoleToml))?;
    set_string(&mut document, "name", fields.display_name.trim());
    set_string(&mut document, "description", fields.purpose.trim());
    set_string(&mut document, "model", fields.model.trim());
    set_string(
        &mut document,
        "model_reasoning_effort",
        fields.effort.trim(),
    );
    set_string(
        &mut document,
        "developer_instructions",
        &fields.instructions,
    );
    Ok(document.to_string())
}

fn set_string(document: &mut DocumentMut, key: &str, value: &str) {
    if let Some(item) = document.get_mut(key) {
        if let Some(existing) = item.as_value() {
            let decor = existing.decor().clone();
            let mut replacement = Value::from(value);
            *replacement.decor_mut() = decor;
            *item = Item::Value(replacement);
            return;
        }
    }
    document[key] = Item::Value(Value::from(value));
}

pub fn render_minimal_role_toml(fields: &RoleEditableFields) -> Result<String, RoleError> {
    validate_display_name(fields.display_name.trim())?;
    validate_purpose(fields.purpose.trim())?;
    validate_selection_criteria(fields.selection_criteria.trim())?;
    validate_model(fields.model.trim())?;
    validate_effort(fields.effort.trim())?;
    validate_instructions(&fields.instructions)?;
    let mut document = DocumentMut::new();
    set_string(&mut document, "name", fields.display_name.trim());
    set_string(&mut document, "description", fields.purpose.trim());
    set_string(&mut document, "model", fields.model.trim());
    set_string(
        &mut document,
        "model_reasoning_effort",
        fields.effort.trim(),
    );
    set_string(
        &mut document,
        "developer_instructions",
        &fields.instructions,
    );
    Ok(document.to_string())
}

#[allow(dead_code)]
pub fn build_managed_role_record(
    id: RoleId,
    fields: &RoleEditableFields,
    order: u32,
    now_ms: u64,
    capabilities: &CapabilitySnapshot,
    description_origin: TextOrigin,
    instructions_origin: TextOrigin,
) -> Result<ManagedRoleRecord, RoleError> {
    validate_editable_fields(fields, capabilities)?;
    let expected_toml = render_minimal_role_toml(fields)?;
    let parsed = parse_role_toml(&expected_toml)?;
    Ok(ManagedRoleRecord {
        id,
        selection_criteria: fields.selection_criteria.trim().to_string(),
        order,
        expected_toml,
        policy_revision: 1,
        policy_hash: policy_hash_for_view(&parsed.view, fields.selection_criteria.trim()),
        policy_updated_at_ms: now_ms,
        description_origin,
        instructions_origin,
    })
}

pub fn policy_hash_for_view(view: &ManagedRoleView, selection_criteria: &str) -> String {
    let mut hasher = Sha256::new();
    for value in [
        view.display_name.as_str(),
        view.purpose.as_str(),
        selection_criteria.trim(),
        view.model.as_str(),
        view.effort.as_str(),
        view.instructions.as_str(),
    ] {
        let bytes = value.as_bytes();
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
#[allow(dead_code)]
pub fn policy_hash_for_toml(text: &str, selection_criteria: &str) -> Result<String, RoleError> {
    let parsed = parse_role_toml(text)?;
    validate_selection_criteria(selection_criteria.trim())?;
    Ok(policy_hash_for_view(&parsed.view, selection_criteria))
}

pub fn sort_managed_role_records(records: &mut [ManagedRoleRecord]) {
    records.sort_by(compare_role_records);
}

fn compare_role_records(left: &ManagedRoleRecord, right: &ManagedRoleRecord) -> Ordering {
    match (left.id.is_default(), right.id.is_default()) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => left
            .order
            .cmp(&right.order)
            .then_with(|| role_id_key(&left.id).cmp(&role_id_key(&right.id))),
    }
}

pub fn reorder_role_records(
    records: &mut Vec<ManagedRoleRecord>,
    requested: &[RoleId],
) -> Result<(), RoleError> {
    validate_managed_role_count(records.len())?;
    validate_unique_role_ids(
        &records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
    )?;
    if requested.len() != records.len() {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleOrder));
    }
    validate_unique_role_ids(requested)?;
    let existing = records
        .iter()
        .map(|record| &record.id)
        .collect::<HashSet<_>>();
    if requested.iter().any(|id| !existing.contains(id)) {
        return Err(RoleError::new(RoleErrorCode::InvalidRoleOrder));
    }
    if requested.first().is_some_and(|id| !id.is_default())
        && records.iter().any(|record| record.id.is_default())
    {
        return Err(RoleError::new(RoleErrorCode::DefaultRoleProtected));
    }
    let mut reordered = Vec::with_capacity(records.len());
    for (index, id) in requested.iter().enumerate() {
        let mut record = records
            .iter()
            .find(|record| record.id == *id)
            .cloned()
            .ok_or_else(|| RoleError::new(RoleErrorCode::InvalidRoleOrder))?;
        record.order = index as u32;
        reordered.push(record);
    }
    *records = reordered;
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
pub fn ordered_role_ids(records: &[ManagedRoleRecord]) -> Result<Vec<RoleId>, RoleError> {
    validate_managed_role_count(records.len())?;
    validate_unique_role_ids(
        &records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut copy = records.to_vec();
    sort_managed_role_records(&mut copy);
    Ok(copy.into_iter().map(|record| record.id).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCapability {
    pub slug: String,
    pub multi_agent_version: String,
    pub supported_reasoning_levels: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelCapabilityWire<'a> {
    slug: &'a str,
    multi_agent_version: &'a str,
    supported_reasoning_levels: Vec<ReasoningLevelWire<'a>>,
}

#[derive(Serialize)]
struct ReasoningLevelWire<'a> {
    effort: &'a str,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ReasoningLevelInput {
    Object { effort: String },
    String(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelCapabilityInput {
    slug: String,
    #[serde(alias = "multi_agent_version")]
    multi_agent_version: String,
    #[serde(alias = "supported_reasoning_levels")]
    supported_reasoning_levels: Vec<ReasoningLevelInput>,
}

impl Serialize for ModelCapability {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ModelCapabilityWire {
            slug: &self.slug,
            multi_agent_version: &self.multi_agent_version,
            supported_reasoning_levels: self
                .supported_reasoning_levels
                .iter()
                .map(|effort| ReasoningLevelWire { effort })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelCapability {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let input = ModelCapabilityInput::deserialize(deserializer)?;
        let efforts = input
            .supported_reasoning_levels
            .into_iter()
            .map(|level| match level {
                ReasoningLevelInput::Object { effort } | ReasoningLevelInput::String(effort) => {
                    effort
                }
            })
            .collect();
        Self::new(input.slug, input.multi_agent_version, efforts).map_err(serde::de::Error::custom)
    }
}

impl ModelCapability {
    pub fn new(
        slug: impl Into<String>,
        multi_agent_version: impl Into<String>,
        supported_reasoning_levels: Vec<String>,
    ) -> Result<Self, RoleError> {
        let capability = Self {
            slug: slug.into(),
            multi_agent_version: multi_agent_version.into(),
            supported_reasoning_levels,
        };
        validate_model_capability(&capability)?;
        Ok(capability)
    }

    pub fn supports_effort(&self, effort: &str) -> bool {
        self.supported_reasoning_levels
            .iter()
            .any(|level| level == effort)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshot {
    pub models: Vec<ModelCapability>,
    /// A stable executable identity (for example a build/version hash), never a
    /// machine-specific path or command output.
    pub executable_identity: String,
    pub fetched_at_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitySnapshotInput {
    models: Vec<ModelCapability>,
    #[serde(alias = "executable_identity")]
    executable_identity: String,
    #[serde(alias = "fetched_at_ms")]
    fetched_at_ms: u64,
}

impl<'de> Deserialize<'de> for CapabilitySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let input = CapabilitySnapshotInput::deserialize(deserializer)?;
        Self::new(input.models, input.executable_identity, input.fetched_at_ms)
            .map_err(serde::de::Error::custom)
    }
}

impl CapabilitySnapshot {
    pub fn new(
        mut models: Vec<ModelCapability>,
        executable_identity: impl Into<String>,
        fetched_at_ms: u64,
    ) -> Result<Self, RoleError> {
        let executable_identity = executable_identity.into();
        if executable_identity.trim().is_empty()
            || executable_identity.contains('\0')
            || executable_identity.contains('/')
            || executable_identity.contains('\\')
        {
            return Err(RoleError::new(RoleErrorCode::InvalidText));
        }
        for model in &models {
            validate_model_capability(model)?;
        }
        models.sort_by(|left, right| left.slug.cmp(&right.slug));
        models.dedup_by(|left, right| left.slug == right.slug);
        Ok(Self {
            models,
            executable_identity,
            fetched_at_ms,
        })
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn available_models(&self) -> Vec<String> {
        self.models
            .iter()
            .filter(|model| model.multi_agent_version == "v2")
            .map(|model| model.slug.clone())
            .collect()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn efforts_for(&self, model: &str) -> Vec<String> {
        self.models
            .iter()
            .find(|capability| capability.slug == model && capability.multi_agent_version == "v2")
            .map(|capability| capability.supported_reasoning_levels.clone())
            .unwrap_or_default()
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn supports(&self, model: &str, effort: &str) -> bool {
        self.models.iter().any(|capability| {
            capability.slug == model
                && capability.multi_agent_version == "v2"
                && capability.supports_effort(effort)
        })
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn validate_selection(&self, model: &str, effort: &str) -> Result<(), RoleError> {
        validate_role_capability_values(model, effort, self)
    }
}

fn validate_model_capability(capability: &ModelCapability) -> Result<(), RoleError> {
    validate_model(&capability.slug)?;
    validate_bounded_text(&capability.multi_agent_version, 1, 32, false)?;
    let mut seen = HashSet::new();
    for effort in &capability.supported_reasoning_levels {
        validate_effort(effort)?;
        if !seen.insert(effort) {
            return Err(RoleError::new(RoleErrorCode::UnsupportedReasoningEffort));
        }
    }
    Ok(())
}

pub fn validate_role_capability(
    view: &ManagedRoleView,
    capabilities: &CapabilitySnapshot,
) -> Result<(), RoleError> {
    validate_role_capability_values(&view.model, &view.effort, capabilities)
}

pub fn validate_role_capability_values(
    model: &str,
    effort: &str,
    capabilities: &CapabilitySnapshot,
) -> Result<(), RoleError> {
    let capability = capabilities
        .models
        .iter()
        .find(|candidate| candidate.slug == model)
        .ok_or_else(|| RoleError::new(RoleErrorCode::UnsupportedModel))?;
    if capability.multi_agent_version != "v2" {
        return Err(RoleError::new(RoleErrorCode::UnsupportedMultiAgentVersion));
    }
    if !capability.supports_effort(effort) {
        return Err(RoleError::new(RoleErrorCode::UnsupportedReasoningEffort));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleSyntaxStatus {
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleCapabilityStatus {
    Supported,
    UnsupportedModel,
    UnsupportedReasoningEffort,
    UnsupportedMultiAgentVersion,
    Unavailable,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDiagnostic {
    pub code: RoleErrorCode,
    pub field: Option<String>,
}

impl From<RoleError> for RoleDiagnostic {
    fn from(error: RoleError) -> Self {
        Self {
            code: error.code,
            field: error.field,
        }
    }
}

/// A discovery summary intentionally contains no document bytes or instructions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredRole {
    pub id: String,
    pub relative_file_name: String,
    pub syntax: RoleSyntaxStatus,
    pub capability: RoleCapabilityStatus,
    pub can_adopt: bool,
    pub managed: bool,
    pub diagnostics: Vec<RoleDiagnostic>,
}

pub fn discover_role(
    relative_file_name: &str,
    text: &str,
    managed_ids: &[RoleId],
    capabilities: Option<&CapabilitySnapshot>,
) -> DiscoveredRole {
    let discovered_role_id = discovered_id(relative_file_name).ok();
    let (id, filename_diagnostic) = match discovered_role_id.as_ref() {
        Some(role_id) => (role_id.as_str().to_string(), None),
        None => (
            safe_discovered_id(relative_file_name),
            Some(RoleError::new(RoleErrorCode::InvalidRoleFileName).into()),
        ),
    };
    let filename_is_valid = filename_diagnostic.is_none();
    let managed = discovered_role_id
        .as_ref()
        .is_some_and(|role_id| managed_ids.iter().any(|candidate| candidate == role_id));
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = filename_diagnostic {
        diagnostics.push(diagnostic);
    }
    let syntax = match parse_role_toml(text) {
        Ok(parsed) => {
            let capability = match capabilities {
                Some(snapshot) => match validate_role_capability(&parsed.view, snapshot) {
                    Ok(()) => RoleCapabilityStatus::Supported,
                    Err(error) => {
                        let code = error.code;
                        diagnostics.push(error.clone().into());
                        capability_status_for_error(code)
                    }
                },
                None => RoleCapabilityStatus::Unavailable,
            };
            if capabilities.is_none() {
                diagnostics.push(RoleDiagnostic {
                    code: RoleErrorCode::CapabilityUnavailable,
                    field: None,
                });
            }
            if !parsed.warnings.is_empty() {
                diagnostics.extend(parsed.warnings.into_iter().map(|warning| RoleDiagnostic {
                    code: match warning.code {
                        RoleWarningCode::UnknownField => RoleErrorCode::UnknownRoleField,
                    },
                    field: warning.field,
                }));
            }
            let can_adopt =
                filename_is_valid && !managed && capability == RoleCapabilityStatus::Supported;
            return DiscoveredRole {
                id,
                relative_file_name: safe_relative_file_name(relative_file_name),
                syntax: RoleSyntaxStatus::Valid,
                capability,
                can_adopt,
                managed,
                diagnostics,
            };
        }
        Err(error) => {
            diagnostics.push(error.into());
            RoleSyntaxStatus::Invalid
        }
    };
    DiscoveredRole {
        id,
        relative_file_name: safe_relative_file_name(relative_file_name),
        syntax,
        capability: RoleCapabilityStatus::Invalid,
        can_adopt: false,
        managed,
        diagnostics,
    }
}

fn discovered_id(relative_file_name: &str) -> Result<RoleId, RoleError> {
    let prefix = format!("{}/", ROLE_DIRECTORY);
    let filename = relative_file_name
        .strip_prefix(&prefix)
        .ok_or_else(|| RoleError::new(RoleErrorCode::InvalidRoleFileName))?;
    role_id_from_file_name(filename)
}

fn safe_discovered_id(relative_file_name: &str) -> String {
    let sanitized = relative_file_name
        .rsplit('/')
        .next()
        .unwrap_or("unknown")
        .strip_suffix(ROLE_EXTENSION)
        .unwrap_or("unknown")
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .take(ROLE_ID_MAX_BYTES)
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn safe_relative_file_name(relative_file_name: &str) -> String {
    let prefix = format!("{}/", ROLE_DIRECTORY);
    if relative_file_name.starts_with(&prefix)
        && !relative_file_name.contains('\\')
        && !relative_file_name.contains('\0')
        && !relative_file_name.contains("..")
    {
        return relative_file_name.to_string();
    }
    let basename = relative_file_name
        .rsplit('/')
        .next()
        .and_then(|name| name.rsplit('\\').next())
        .filter(|name| {
            !name.is_empty() && *name != "." && *name != ".." && !name.contains(['\0', ':'])
        });
    basename
        .map(str::to_string)
        .unwrap_or_else(|| "<invalid>".to_string())
}

fn capability_status_for_error(code: RoleErrorCode) -> RoleCapabilityStatus {
    match code {
        RoleErrorCode::UnsupportedModel => RoleCapabilityStatus::UnsupportedModel,
        RoleErrorCode::UnsupportedReasoningEffort => {
            RoleCapabilityStatus::UnsupportedReasoningEffort
        }
        RoleErrorCode::UnsupportedMultiAgentVersion => {
            RoleCapabilityStatus::UnsupportedMultiAgentVersion
        }
        RoleErrorCode::CapabilityUnavailable => RoleCapabilityStatus::Unavailable,
        _ => RoleCapabilityStatus::Invalid,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleMigrationChoice {
    AdoptLive,
    AdoptStored,
    UseSafeDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleMigrationSource {
    Live,
    Stored,
    SafeDefault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleMigrationPendingReason {
    MissingStored,
    MissingLive,
    InvalidStored,
    InvalidLive,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoleMigrationPlan {
    Auto {
        source: RoleMigrationSource,
        lifecycle: RoleLifecycle,
    },
    Pending {
        reason: RoleMigrationPendingReason,
        choices: Vec<RoleMigrationChoice>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleMigrationResult {
    pub record: ManagedRoleRecord,
    pub lifecycle: RoleLifecycle,
    pub source: RoleMigrationSource,
}

/// Plan migration of the legacy whole-file `agents-default-toml` owner.
///
/// The function is pure: it parses supplied bytes only and never writes either
/// source.  A valid/equal pair is safe to auto-adopt; every other case is pending
/// and must be resolved by one of the three explicit choices.
pub fn plan_default_role_migration(
    stored_expected: Option<&str>,
    live_toml: Option<&str>,
    previous_lifecycle: RoleLifecycle,
    safe_default_toml: &str,
) -> Result<RoleMigrationPlan, RoleError> {
    let stored = parse_migration_source(stored_expected);
    let live = parse_migration_source(live_toml);
    let safe = parse_role_toml(safe_default_toml).map(|_| ());
    if stored.is_ok() && live.is_ok() && sources_equal(stored_expected, live_toml) {
        return Ok(RoleMigrationPlan::Auto {
            source: RoleMigrationSource::Stored,
            lifecycle: previous_lifecycle,
        });
    }

    let reason = match (stored_expected, live_toml, stored.is_ok(), live.is_ok()) {
        (None, _, _, _) => RoleMigrationPendingReason::MissingStored,
        (_, None, _, _) => RoleMigrationPendingReason::MissingLive,
        (_, _, false, true) => RoleMigrationPendingReason::InvalidStored,
        (_, _, true, false) => RoleMigrationPendingReason::InvalidLive,
        _ => RoleMigrationPendingReason::Conflict,
    };
    let mut choices = Vec::new();
    if live.is_ok() {
        choices.push(RoleMigrationChoice::AdoptLive);
    }
    if stored.is_ok() {
        choices.push(RoleMigrationChoice::AdoptStored);
    }
    if safe.is_ok() {
        choices.push(RoleMigrationChoice::UseSafeDefault);
    }
    Ok(RoleMigrationPlan::Pending { reason, choices })
}

pub fn resolve_default_role_migration(
    choice: RoleMigrationChoice,
    stored_expected: Option<&str>,
    live_toml: Option<&str>,
    safe_default_toml: &str,
    previous_lifecycle: RoleLifecycle,
    now_ms: u64,
) -> Result<RoleMigrationResult, RoleError> {
    let (source, selected) = match choice {
        RoleMigrationChoice::AdoptLive => (
            RoleMigrationSource::Live,
            live_toml.ok_or_else(|| RoleError::new(RoleErrorCode::MigrationSourceUnavailable))?,
        ),
        RoleMigrationChoice::AdoptStored => (
            RoleMigrationSource::Stored,
            stored_expected
                .ok_or_else(|| RoleError::new(RoleErrorCode::MigrationSourceUnavailable))?,
        ),
        RoleMigrationChoice::UseSafeDefault => {
            (RoleMigrationSource::SafeDefault, safe_default_toml)
        }
    };
    let parsed = parse_role_toml(selected).map_err(|_| {
        RoleError::new(if source == RoleMigrationSource::SafeDefault {
            RoleErrorCode::SafeDefaultInvalid
        } else {
            RoleErrorCode::MigrationSourceInvalid
        })
    })?;
    let live_matches = live_toml.is_some_and(|live| sources_equal(Some(selected), Some(live)));
    let lifecycle = if previous_lifecycle == RoleLifecycle::Locked && !live_matches {
        RoleLifecycle::Disabled
    } else {
        previous_lifecycle
    };
    let record = ManagedRoleRecord {
        id: RoleId::new(DEFAULT_ROLE_ID).expect("default role id is valid"),
        selection_criteria: DEFAULT_SELECTION_CRITERIA.to_string(),
        order: 0,
        expected_toml: selected.to_string(),
        policy_revision: 1,
        policy_hash: policy_hash_for_view(&parsed.view, DEFAULT_SELECTION_CRITERIA),
        policy_updated_at_ms: now_ms,
        description_origin: TextOrigin::Builtin,
        instructions_origin: TextOrigin::Builtin,
    };
    Ok(RoleMigrationResult {
        record,
        lifecycle,
        source,
    })
}

fn parse_migration_source(source: Option<&str>) -> Result<ParsedRoleToml, RoleError> {
    source
        .ok_or_else(|| RoleError::new(RoleErrorCode::MigrationSourceUnavailable))
        .and_then(parse_role_toml)
}

fn sources_equal(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            if left.trim() != right.trim() {
                return false;
            }
            match (parse_role_toml(left), parse_role_toml(right)) {
                (Ok(left), Ok(right)) => {
                    left.view.display_name == right.view.display_name
                        && left.view.purpose == right.view.purpose
                        && left.view.model == right.view.model
                        && left.view.effort == right.view.effort
                        && left.view.instructions == right.view.instructions
                        && left.warnings == right.warnings
                }
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> CapabilitySnapshot {
        CapabilitySnapshot::new(
            vec![
                ModelCapability::new("model-v2", "v2", vec!["low".into(), "high".into()]).unwrap(),
                ModelCapability::new("model-v1", "v1", vec!["low".into()]).unwrap(),
            ],
            "fixture-build",
            10,
        )
        .unwrap()
    }

    fn role_toml(model: &str, effort: &str) -> String {
        format!(
            "# keep this comment\nname = \"Fixture\" # keep name note\ndescription = \"Fixture role\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"{effort}\"\ndeveloper_instructions = \"fixture instructions\"\nunknown = 1\n"
        )
    }

    #[test]
    fn role_id_boundaries_and_path_validation_are_strict() {
        assert!(RoleId::new("a").is_ok());
        assert!(RoleId::new(&format!("a{}", "x".repeat(62))).is_ok());
        assert!(RoleId::new(&format!("a{}", "x".repeat(63))).is_err());
        for invalid in ["", "A", "-x", "x_", "x/y", "x\\y", "x..y", "../x", "x\0y"] {
            assert!(
                RoleId::new(invalid).is_err(),
                "{invalid:?} should be rejected"
            );
        }
        let id = RoleId::new("reviewer").unwrap();
        assert_eq!(role_file_name(&id), "reviewer.toml");
        assert_eq!(role_relative_path(&id), "agents/reviewer.toml");
        assert!(validate_role_file_name(&id, "reviewer.toml").is_ok());
        assert!(validate_role_file_name(&id, "../reviewer.toml").is_err());
        assert!(validate_role_relative_path(&id, "agents/../reviewer.toml").is_err());
    }

    #[test]
    fn duplicate_ids_are_case_insensitive_and_capacity_has_warning_threshold() {
        let ids = vec![
            RoleId::new("reviewer").unwrap(),
            RoleId::new("writer").unwrap(),
        ];
        assert!(validate_unique_role_ids(&ids).is_ok());
        assert!(validate_role_count(32).is_ok());
        assert!(validate_role_count(33).is_err());
        assert!(!role_count_near_limit(27));
        assert!(role_count_near_limit(28));
        assert!(role_count_near_limit(32));
        assert!(can_add_managed_role(31).is_ok());
        assert!(can_add_managed_role(32).is_err());
    }

    #[test]
    fn default_is_protected_but_other_roles_can_be_changed() {
        let default = RoleId::new(DEFAULT_ROLE_ID).unwrap();
        let custom = RoleId::new("reviewer").unwrap();
        assert!(validate_new_role_id("new-role", std::slice::from_ref(&default)).is_ok());
        assert_eq!(
            validate_new_role_id("default", &[]).unwrap_err().code,
            RoleErrorCode::DefaultRoleProtected
        );
        for action in [
            RoleAction::Rename,
            RoleAction::CopyInto,
            RoleAction::StopManaging,
            RoleAction::Delete,
        ] {
            assert_eq!(
                validate_default_action(&default, action).unwrap_err().code,
                RoleErrorCode::DefaultRoleProtected
            );
            assert!(validate_default_action(&custom, action).is_ok());
        }
        assert!(validate_copy_roles(&default, &custom).is_ok());
        assert_eq!(
            validate_copy_roles(&custom, &default).unwrap_err().code,
            RoleErrorCode::DefaultRoleProtected
        );
    }

    #[test]
    fn lifecycle_is_a_valid_enum_and_rejects_legacy_impossible_state() {
        assert_eq!(
            RoleLifecycle::from_flags(false, false).unwrap(),
            RoleLifecycle::Disabled
        );
        assert_eq!(
            RoleLifecycle::from_flags(true, false).unwrap(),
            RoleLifecycle::Applied
        );
        assert_eq!(
            RoleLifecycle::from_flags(true, true).unwrap(),
            RoleLifecycle::Locked
        );
        assert!(RoleLifecycle::from_flags(false, true).is_err());
        assert_eq!(
            RoleLifecycle::Disabled
                .transition(RoleLifecycleAction::Lock)
                .unwrap(),
            RoleLifecycle::Locked
        );
        assert_eq!(
            RoleLifecycle::Locked
                .transition(RoleLifecycleAction::Disable)
                .unwrap(),
            RoleLifecycle::Disabled
        );
    }

    #[test]
    fn ordering_keeps_default_first_and_does_not_change_ids_or_paths() {
        let make = |id: &str, order| ManagedRoleRecord {
            id: RoleId::new(id).unwrap(),
            selection_criteria: "fixture criteria".into(),
            order,
            expected_toml: role_toml("model-v2", "low"),
            policy_revision: 1,
            policy_hash: "hash".into(),
            policy_updated_at_ms: 1,
            description_origin: TextOrigin::User,
            instructions_origin: TextOrigin::User,
        };
        let mut records = vec![make("writer", 2), make("default", 9), make("reviewer", 1)];
        sort_managed_role_records(&mut records);
        assert_eq!(
            records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            vec!["default", "reviewer", "writer"]
        );
        assert_eq!(role_relative_path(&records[0].id), "agents/default.toml");
        let requested = vec![
            RoleId::new("default").unwrap(),
            RoleId::new("writer").unwrap(),
            RoleId::new("reviewer").unwrap(),
        ];
        reorder_role_records(&mut records, &requested).unwrap();
        assert_eq!(records[1].id.as_str(), "writer");
        assert_eq!(records[1].order, 1);
        assert_eq!(role_relative_path(&records[1].id), "agents/writer.toml");
    }

    #[test]
    fn role_toml_parses_known_fields_and_only_warns_on_unknown_keys() {
        let parsed = parse_role_toml(&role_toml("model-v2", "low")).unwrap();
        assert_eq!(parsed.view.display_name, "Fixture");
        assert_eq!(parsed.view.purpose, "Fixture role");
        assert_eq!(parsed.view.model, "model-v2");
        assert_eq!(parsed.view.effort, "low");
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.warnings[0].field.as_deref(), Some("unknown"));
        assert!(parse_role_toml("name = 1").is_err());
        assert!(parse_role_toml("name = \"x\"\0").is_err());
    }

    #[test]
    fn updating_known_fields_preserves_comments_and_unknown_keys() {
        let original = role_toml("model-v2", "low");
        let fields = RoleEditableFields {
            display_name: "Updated".into(),
            purpose: "Updated purpose".into(),
            selection_criteria: "Updated criteria".into(),
            model: "model-v2".into(),
            effort: "high".into(),
            instructions: "updated instructions".into(),
        };
        let updated = update_role_toml(&original, &fields).unwrap();
        assert!(updated.contains("# keep this comment"));
        assert!(updated.contains("# keep name note"));
        assert!(updated.contains("unknown = 1"));
        assert_eq!(parse_role_toml(&updated).unwrap().view.effort, "high");
        assert_ne!(
            policy_hash_for_toml(&original, "fixture criteria").unwrap(),
            policy_hash_for_toml(&updated, "Updated criteria").unwrap()
        );
        let formatting_only = original.replace("# keep this comment\n", "# another comment\n");
        assert_eq!(
            policy_hash_for_toml(&original, "fixture criteria").unwrap(),
            policy_hash_for_toml(&formatting_only, "fixture criteria").unwrap()
        );
    }

    #[test]
    fn capability_only_accepts_v2_models_and_declared_efforts() {
        let snapshot = capabilities();
        assert_eq!(snapshot.available_models(), vec!["model-v2"]);
        assert_eq!(snapshot.efforts_for("model-v2"), vec!["low", "high"]);
        assert!(snapshot.supports("model-v2", "high"));
        assert!(!snapshot.supports("model-v1", "low"));
        assert_eq!(
            snapshot
                .validate_selection("missing", "low")
                .unwrap_err()
                .code,
            RoleErrorCode::UnsupportedModel
        );
        assert_eq!(
            snapshot
                .validate_selection("model-v1", "low")
                .unwrap_err()
                .code,
            RoleErrorCode::UnsupportedMultiAgentVersion
        );
        assert_eq!(
            snapshot
                .validate_selection("model-v2", "medium")
                .unwrap_err()
                .code,
            RoleErrorCode::UnsupportedReasoningEffort
        );
        let json = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(
            json["models"][0]["supportedReasoningLevels"][0]["effort"],
            "low"
        );
        let object_fixture = serde_json::json!({
            "models": [{
                "slug": "fixture-v2",
                "multiAgentVersion": "v2",
                "supportedReasoningLevels": [{ "effort": "medium" }]
            }],
            "executableIdentity": "fixture-build",
            "fetchedAtMs": 20
        });
        let decoded: CapabilitySnapshot = serde_json::from_value(object_fixture).unwrap();
        assert!(decoded.supports("fixture-v2", "medium"));
        let path_identity = serde_json::json!({
            "models": [],
            "executableIdentity": "/private/codex",
            "fetchedAtMs": 20
        });
        assert!(serde_json::from_value::<CapabilitySnapshot>(path_identity).is_err());
    }

    #[test]
    fn discovered_summary_has_no_document_or_instruction_fields() {
        let snapshot = capabilities();
        let discovered = discover_role(
            "agents/discovered.toml",
            &role_toml("model-v2", "low"),
            &[],
            Some(&snapshot),
        );
        assert!(discovered.can_adopt);
        let serialized = serde_json::to_string(&discovered).unwrap();
        assert!(!serialized.contains("fixture instructions"));
        assert!(!serialized.contains("developer_instructions"));
        assert!(!serialized.contains("/Users/"));
        let outside = discover_role(
            "/private/user/codex/agents/discovered.toml",
            &role_toml("model-v2", "low"),
            &[],
            Some(&snapshot),
        );
        assert_eq!(outside.relative_file_name, "discovered.toml");
        assert!(!serde_json::to_string(&outside)
            .unwrap()
            .contains("/private/"));
        let managed_invalid = discover_role(
            "agents/discovered.toml",
            "name = 1",
            &[RoleId::new("discovered").unwrap()],
            Some(&snapshot),
        );
        assert!(managed_invalid.managed);
        assert!(!managed_invalid.can_adopt);
    }

    #[test]
    fn migration_auto_adopts_equal_sources_and_has_three_explicit_resolution_branches() {
        let stored = role_toml("model-v2", "low");
        let live = role_toml("model-v2", "high");
        let safe = role_toml("model-v2", "low");
        let plan =
            plan_default_role_migration(Some(&stored), Some(&stored), RoleLifecycle::Locked, &safe)
                .unwrap();
        assert_eq!(
            plan,
            RoleMigrationPlan::Auto {
                source: RoleMigrationSource::Stored,
                lifecycle: RoleLifecycle::Locked,
            }
        );

        let conflict =
            plan_default_role_migration(Some(&stored), Some(&live), RoleLifecycle::Locked, &safe)
                .unwrap();
        assert!(matches!(conflict, RoleMigrationPlan::Pending { .. }));

        let adopted_live = resolve_default_role_migration(
            RoleMigrationChoice::AdoptLive,
            Some(&stored),
            Some(&live),
            &safe,
            RoleLifecycle::Locked,
            20,
        )
        .unwrap();
        assert_eq!(adopted_live.lifecycle, RoleLifecycle::Locked);
        assert_eq!(adopted_live.source, RoleMigrationSource::Live);

        let adopted_stored = resolve_default_role_migration(
            RoleMigrationChoice::AdoptStored,
            Some(&stored),
            Some(&live),
            &safe,
            RoleLifecycle::Locked,
            20,
        )
        .unwrap();
        assert_eq!(adopted_stored.lifecycle, RoleLifecycle::Disabled);
        assert_eq!(adopted_stored.record.id.as_str(), DEFAULT_ROLE_ID);

        let safe_default = resolve_default_role_migration(
            RoleMigrationChoice::UseSafeDefault,
            Some(&stored),
            Some(&live),
            &safe,
            RoleLifecycle::Locked,
            20,
        )
        .unwrap();
        assert_eq!(safe_default.lifecycle, RoleLifecycle::Disabled);
        assert_eq!(safe_default.source, RoleMigrationSource::SafeDefault);
    }
}
