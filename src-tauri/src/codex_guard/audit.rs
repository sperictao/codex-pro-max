//! Runtime provenance and operation-audit primitives.
//!
//! This module deliberately has no knowledge of the Guard coordinator or SQLite.  It only
//! accepts bounded, already-read bytes and returns a small allow-listed DTO.  The parser never
//! stores a prompt, message, path, database body, or an error string.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::io::Read;

pub const AUDIT_SCHEMA_VERSION: u32 = 1;
pub const MAX_ROLLOUT_DATES: usize = 7;
pub const MAX_ROLLOUT_FILES: usize = 500;
pub const MAX_ROLLOUT_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ROLLOUT_LINE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CHILDREN: usize = 20;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAX_PAIR_DELAY_MS: u64 = 120_000;
pub const MAX_OPERATION_AUDIT_ENTRIES: usize = 500;
pub const OPERATION_AUDIT_RETENTION_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

/// The source layer observed by the audit.  `RolloutOnly` is intentionally not green: sampling
/// evidence from the SQLite layer is required before a complete result can be claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AuditSourceSupport {
    Unsupported,
    RolloutOnly,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    Match,
    Missing,
    Incomplete,
    Unsupported,
    Ambiguous,
    Mismatch,
}

impl AuditVerdict {
    fn rank(self) -> u8 {
        match self {
            // Keep the order explicit.  A caller must never accidentally make an ambiguous
            // result look like a match by sorting enum discriminants.
            Self::Mismatch => 6,
            Self::Ambiguous => 5,
            Self::Unsupported => 4,
            Self::Incomplete => 3,
            Self::Missing => 2,
            Self::Match => 1,
        }
    }

    pub(crate) fn worst(self, other: Self) -> Self {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

/// A notice has a stable code only.  It intentionally does not contain parser error text or
/// any source identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AuditNotice {
    pub code: String,
}

impl AuditNotice {
    #[cfg_attr(test, allow(dead_code))]
    pub fn new(code: &'static str) -> Self {
        Self {
            code: code.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ParentDispatchAudit {
    pub verdict: AuditVerdict,
    pub observed_agent_type: Option<String>,
    pub observed_fork_turns: Option<String>,
    pub override_model: Option<String>,
    pub override_effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TurnAudit {
    pub turn_id: String,
    pub started_at_ms: Option<u64>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub multi_agent_version: Option<String>,
    pub client_verdict: AuditVerdict,
    pub outbound_requested_model: Option<String>,
    pub outbound_evidence_count: u32,
    pub outbound_verdict: AuditVerdict,
    pub verdict: AuditVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentAudit {
    pub thread_id: String,
    pub role_id: String,
    pub expected_policy_revision: Option<u64>,
    pub expected_policy_hash: Option<String>,
    pub expected_model: Option<String>,
    pub expected_effort: Option<String>,
    pub parent: ParentDispatchAudit,
    pub turns: Vec<TurnAudit>,
    pub verdict: AuditVerdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SubagentAuditResult {
    pub schema_version: u32,
    pub checked_at_ms: u64,
    pub source_support: AuditSourceSupport,
    pub agents: Vec<AgentAudit>,
    pub notices: Vec<AuditNotice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRoleSnapshot {
    pub role_id: String,
    pub policy_revision: Option<u64>,
    pub policy_hash: Option<String>,
    pub expected_model: Option<String>,
    pub expected_effort: Option<String>,
    pub policy_updated_at_ms: Option<u64>,
    pub managed: bool,
}

impl ManagedRoleSnapshot {
    #[cfg(test)]
    pub fn new(
        role_id: impl Into<String>,
        expected_model: impl Into<String>,
        expected_effort: impl Into<String>,
    ) -> Self {
        Self {
            role_id: role_id.into(),
            policy_revision: None,
            policy_hash: None,
            expected_model: Some(expected_model.into()),
            expected_effort: Some(expected_effort.into()),
            policy_updated_at_ms: None,
            managed: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditLimits {
    pub max_date_directories: usize,
    pub max_rollout_files: usize,
    pub max_file_bytes: usize,
    pub max_line_bytes: usize,
    pub max_children: usize,
    pub max_pair_delay_ms: u64,
    pub max_identifier_bytes: usize,
}

impl Default for AuditLimits {
    fn default() -> Self {
        Self {
            max_date_directories: MAX_ROLLOUT_DATES,
            max_rollout_files: MAX_ROLLOUT_FILES,
            max_file_bytes: MAX_ROLLOUT_FILE_BYTES,
            max_line_bytes: MAX_ROLLOUT_LINE_BYTES,
            max_children: MAX_CHILDREN,
            max_pair_delay_ms: MAX_PAIR_DELAY_MS,
            max_identifier_bytes: MAX_IDENTIFIER_BYTES,
        }
    }
}

/// A caller supplies a relative/date sorting key, never the parser.  The key is used solely to
/// choose the newest bounded inputs and is never copied into an audit result or notice.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub struct RolloutFile {
    pub date_key: String,
    pub file_key: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct ParsedRollout {
    pub observations: Vec<RolloutObservation>,
    pub notices: Vec<AuditNotice>,
    pub truncated: bool,
}

impl fmt::Debug for ParsedRollout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedRollout")
            .field("observation_count", &self.observations.len())
            .field(
                "notice_codes",
                &self.notices.iter().map(|n| &n.code).collect::<Vec<_>>(),
            )
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum RolloutObservation {
    Session(SessionObservation),
    Turn(TurnObservation),
    Spawn(SpawnObservation),
}

impl fmt::Debug for RolloutObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(value) => f.debug_tuple("Session").field(value).finish(),
            Self::Turn(value) => f.debug_tuple("Turn").field(value).finish(),
            Self::Spawn(value) => f.debug_tuple("Spawn").field(value).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SessionObservation {
    pub thread_id: String,
    pub parent_thread_id: Option<String>,
    pub agent_role: Option<String>,
    pub started_at_ms: Option<u64>,
    join_key: Option<String>,
    time_state: TimeState,
    // `sub_agent_activity` records the parent-side launch instant.  A child rollout then emits
    // a second, richer session record; retaining the source lets merge keep the launch instant
    // for deterministic spawn pairing while enriching it with the child's role metadata.
    activity_start: bool,
}

impl fmt::Debug for SessionObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionObservation")
            .field("thread_id", &self.thread_id)
            .field("parent_thread_id", &self.parent_thread_id)
            .field("agent_role", &self.agent_role)
            .field("started_at_ms", &self.started_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct TurnObservation {
    pub thread_id: String,
    pub turn_id: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub multi_agent_version: Option<String>,
    pub started_at_ms: Option<u64>,
    time_state: TimeState,
}

impl fmt::Debug for TurnObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TurnObservation")
            .field("thread_id", &self.thread_id)
            .field("turn_id", &self.turn_id)
            .field("model", &self.model)
            .field("effort", &self.effort)
            .field("multi_agent_version", &self.multi_agent_version)
            .field("started_at_ms", &self.started_at_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SpawnObservation {
    pub parent_thread_id: String,
    pub task_name_present: bool,
    pub observed_agent_type: Option<String>,
    pub observed_fork_turns: Option<String>,
    pub override_model: Option<String>,
    pub override_effort: Option<String>,
    pub spawned_at_ms: Option<u64>,
    task_key: Option<String>,
    task_key_valid: bool,
    agent_type_present: bool,
    fork_turns_present: bool,
    model_present: bool,
    effort_present: bool,
    fields_valid: bool,
    time_state: TimeState,
}

impl fmt::Debug for SpawnObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpawnObservation")
            .field("parent_thread_id", &self.parent_thread_id)
            .field("task_name_present", &self.task_name_present)
            .field("observed_agent_type", &self.observed_agent_type)
            .field("observed_fork_turns", &self.observed_fork_turns)
            .field("override_model", &self.override_model)
            .field("override_effort", &self.override_effort)
            .field("spawned_at_ms", &self.spawned_at_ms)
            .finish()
    }
}

impl SpawnObservation {
    /// Constructor for integration tests and the SQLite adapter.  `task_name` is retained only
    /// as an internal join key and is never serialized by this module.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        parent_thread_id: impl Into<String>,
        task_name: Option<impl Into<String>>,
        agent_type: Option<impl Into<String>>,
        fork_turns: Option<impl Into<String>>,
        model: Option<impl Into<String>>,
        effort: Option<impl Into<String>>,
        spawned_at_ms: Option<u64>,
    ) -> Self {
        let task = task_name.map(Into::into);
        let agent = agent_type.map(Into::into);
        let fork = fork_turns.map(Into::into);
        let model = model.map(Into::into);
        let effort = effort.map(Into::into);
        let agent_type_present = agent.is_some();
        let fork_turns_present = fork.is_some();
        let model_present = model.is_some();
        let effort_present = effort.is_some();
        Self {
            parent_thread_id: parent_thread_id.into(),
            task_name_present: task.is_some(),
            observed_agent_type: agent,
            observed_fork_turns: fork,
            override_model: model,
            override_effort: effort,
            spawned_at_ms,
            task_key: task,
            task_key_valid: true,
            agent_type_present,
            fork_turns_present,
            model_present,
            effort_present,
            fields_valid: true,
            time_state: spawned_at_ms.map_or(TimeState::Missing, TimeState::Valid),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeState {
    Missing,
    Valid(u64),
    Invalid,
}

#[derive(Clone, PartialEq, Eq)]
struct TextField {
    present: bool,
    value: Option<String>,
    valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventTime {
    Missing,
    Valid(u64),
    Invalid,
}

/// Parse one bounded JSONL rollout stream.  The implementation streams chunks into a capped
/// line buffer, so a hostile line cannot allocate unbounded memory.
pub fn parse_rollout_jsonl<R: Read>(reader: R, limits: AuditLimits) -> ParsedRollout {
    let mut parser = JsonlParser::new(reader, limits);
    parser.run();
    parser.finish()
}

#[cfg(test)]
pub fn parse_rollout_bytes(bytes: &[u8], limits: AuditLimits) -> ParsedRollout {
    let reached_file_limit = limits.max_file_bytes > 0 && bytes.len() >= limits.max_file_bytes;
    let mut parsed = parse_rollout_jsonl(std::io::Cursor::new(bytes), limits);
    if reached_file_limit {
        parsed.truncated = true;
        push_notice(&mut parsed.notices, "audit_truncated");
    }
    parsed
}

/// Parse files in deterministic newest-first order.  Date and file keys are sorting hints only;
/// they never appear in the returned notices or DTO.
#[cfg_attr(test, allow(dead_code))]
#[cfg(test)]
pub fn parse_rollout_files(mut files: Vec<RolloutFile>, limits: AuditLimits) -> ParsedRollout {
    files.sort_by(|left, right| {
        right
            .date_key
            .cmp(&left.date_key)
            .then_with(|| right.file_key.cmp(&left.file_key))
    });

    let mut selected_dates = BTreeSet::new();
    let mut parsed = ParsedRollout {
        observations: Vec::new(),
        notices: Vec::new(),
        truncated: false,
    };
    let mut selected_files = 0usize;
    for file in files {
        if selected_files >= limits.max_rollout_files {
            parsed.truncated = true;
            push_notice(&mut parsed.notices, "audit_truncated");
            break;
        }
        if !selected_dates.contains(&file.date_key) {
            if selected_dates.len() >= limits.max_date_directories {
                parsed.truncated = true;
                push_notice(&mut parsed.notices, "audit_truncated");
                break;
            }
            selected_dates.insert(file.date_key.clone());
        }
        let item = parse_rollout_bytes(&file.bytes, limits);
        parsed.observations.extend(item.observations);
        parsed.truncated |= item.truncated;
        for notice in item.notices {
            push_notice(&mut parsed.notices, &notice.code);
        }
        selected_files += 1;
    }
    // Reaching a production budget is itself an incomplete boundary: without scanning one
    // additional entry we cannot prove that the bounded view is the complete source.
    if (limits.max_rollout_files > 0 && selected_files >= limits.max_rollout_files)
        || (limits.max_date_directories > 0 && selected_dates.len() >= limits.max_date_directories)
    {
        parsed.truncated = true;
        push_notice(&mut parsed.notices, "audit_truncated");
    }
    parsed
}

/// Merge parsed rollout observations against the immutable role policy snapshot taken at audit
/// start.  This is deterministic and never picks a nearest/latest candidate when evidence is
/// conflicting.
pub fn merge_rollout_evidence(
    parsed: &ParsedRollout,
    roles: &[ManagedRoleSnapshot],
    checked_at_ms: u64,
) -> SubagentAuditResult {
    merge_rollout_evidence_with_limits(parsed, roles, checked_at_ms, AuditLimits::default())
}

/// Variant used by callers that intentionally use a tighter test/rollout budget.  The default
/// entry point remains fixed to the Plan 004 production bounds.
pub fn merge_rollout_evidence_with_limits(
    parsed: &ParsedRollout,
    roles: &[ManagedRoleSnapshot],
    checked_at_ms: u64,
    limits: AuditLimits,
) -> SubagentAuditResult {
    let mut notices = parsed.notices.clone();
    let mut role_map = BTreeMap::new();
    let mut invalid_policy_roles = HashSet::new();
    for role in roles {
        if valid_identifier(&role.role_id, MAX_IDENTIFIER_BYTES) {
            let mut safe_role = role.clone();
            if let Some(hash) = safe_role.policy_hash.as_deref() {
                if !valid_hash(hash) {
                    safe_role.policy_hash = None;
                    invalid_policy_roles.insert(role.role_id.clone());
                    push_notice(&mut notices, "audit_bad_policy_hash");
                }
            }
            if safe_role
                .expected_model
                .as_deref()
                .is_some_and(|value| !valid_identifier(value, MAX_IDENTIFIER_BYTES))
            {
                safe_role.expected_model = None;
                invalid_policy_roles.insert(role.role_id.clone());
                push_notice(&mut notices, "audit_bad_identifier");
            }
            if safe_role
                .expected_effort
                .as_deref()
                .is_some_and(|value| !valid_identifier(value, MAX_IDENTIFIER_BYTES))
            {
                safe_role.expected_effort = None;
                invalid_policy_roles.insert(role.role_id.clone());
                push_notice(&mut notices, "audit_bad_identifier");
            }
            if role_map
                .get(&role.role_id)
                .is_some_and(|previous| previous != &safe_role)
            {
                invalid_policy_roles.insert(role.role_id.clone());
                push_notice(&mut notices, "audit_ambiguous");
            }
            role_map.insert(role.role_id.clone(), safe_role);
        } else {
            push_notice(&mut notices, "audit_bad_identifier");
        }
    }

    let mut children: BTreeMap<String, ChildEvidence> = BTreeMap::new();
    let mut spawns: BTreeMap<(String, String), Vec<SpawnObservation>> = BTreeMap::new();
    for observation in &parsed.observations {
        match observation {
            RolloutObservation::Session(session) => {
                if !valid_identifier(&session.thread_id, MAX_IDENTIFIER_BYTES) {
                    push_notice(&mut notices, "audit_bad_identifier");
                    continue;
                }
                let entry = children
                    .entry(session.thread_id.clone())
                    .or_insert_with(|| ChildEvidence::new(session.thread_id.clone()));
                if let Some(previous) = entry.session.as_ref() {
                    if let Some(merged) = merge_session_observation(previous, session) {
                        entry.session = Some(merged);
                    } else {
                        entry.session_conflict = true;
                        push_notice(&mut notices, "audit_ambiguous");
                    }
                } else {
                    entry.session = Some(session.clone());
                }
            }
            RolloutObservation::Turn(turn) => {
                if !valid_identifier(&turn.thread_id, MAX_IDENTIFIER_BYTES)
                    || !valid_identifier(&turn.turn_id, MAX_IDENTIFIER_BYTES)
                {
                    push_notice(&mut notices, "audit_bad_identifier");
                    continue;
                }
                let entry = children
                    .entry(turn.thread_id.clone())
                    .or_insert_with(|| ChildEvidence::new(turn.thread_id.clone()));
                let turns = entry.turns.entry(turn.turn_id.clone()).or_default();
                if !turns.iter().any(|previous| turn_equivalent(previous, turn)) {
                    if !turns.is_empty() {
                        entry.turn_conflict.insert(turn.turn_id.clone());
                        push_notice(&mut notices, "audit_ambiguous");
                    }
                    turns.push(turn.clone());
                }
            }
            RolloutObservation::Spawn(spawn) => {
                if !valid_identifier(&spawn.parent_thread_id, MAX_IDENTIFIER_BYTES) {
                    push_notice(&mut notices, "audit_bad_identifier");
                    continue;
                }
                if let (Some(task_key), true) = (&spawn.task_key, spawn.task_key_valid) {
                    if valid_join_key(task_key, MAX_IDENTIFIER_BYTES) {
                        spawns
                            .entry((spawn.parent_thread_id.clone(), task_key.clone()))
                            .or_default()
                            .push(spawn.clone());
                    } else {
                        push_notice(&mut notices, "audit_bad_identifier");
                    }
                } else {
                    push_notice(&mut notices, "audit_missing_spawn");
                }
            }
        }
    }

    let mut child_key_counts: HashMap<(String, String), usize> = HashMap::new();
    for child in children.values() {
        if let Some(session) = child.session.as_ref() {
            if let (Some(parent), Some(task)) =
                (session.parent_thread_id.as_ref(), session.join_key.as_ref())
            {
                *child_key_counts
                    .entry((parent.clone(), task.clone()))
                    .or_default() += 1;
            }
        }
    }
    let mut quantity_conflicts = HashSet::new();
    for (key, candidates) in &spawns {
        if let Some(child_count) = child_key_counts.get(key) {
            if *child_count != candidates.len() {
                quantity_conflicts.insert(key.clone());
                push_notice(&mut notices, "audit_ambiguous");
            }
        }
    }

    // A rollout file also contains the parent thread's session_meta and turns.  Only sessions
    // with an explicit parent are child audits; root records must never become a synthetic role.
    let mut eligible_children = children
        .values()
        .filter(|child| {
            child
                .session
                .as_ref()
                .is_some_and(|session| session.parent_thread_id.is_some())
        })
        .collect::<Vec<_>>();
    eligible_children.sort_by(|left, right| {
        let left_started = left
            .session
            .as_ref()
            .and_then(|session| session.started_at_ms);
        let right_started = right
            .session
            .as_ref()
            .and_then(|session| session.started_at_ms);
        right_started
            .cmp(&left_started)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let eligible_count = eligible_children.len();
    let mut agents_with_start = Vec::new();
    for child in eligible_children.iter().take(limits.max_children) {
        let started_at_ms = child
            .session
            .as_ref()
            .and_then(|session| session.started_at_ms);
        agents_with_start.push((
            started_at_ms,
            build_agent_audit(
                child,
                &spawns,
                &quantity_conflicts,
                &role_map,
                &invalid_policy_roles,
                &mut notices,
                limits.max_pair_delay_ms,
            ),
        ));
    }
    if (limits.max_children > 0 && eligible_count >= limits.max_children)
        || (limits.max_children == 0 && eligible_count > 0)
    {
        push_notice(&mut notices, "audit_truncated");
        for (_, agent) in &mut agents_with_start {
            agent.verdict = agent.verdict.worst(AuditVerdict::Incomplete);
        }
    }
    if parsed.truncated {
        push_notice(&mut notices, "audit_truncated");
        for (_, agent) in &mut agents_with_start {
            agent.verdict = agent.verdict.worst(AuditVerdict::Incomplete);
        }
    }
    if notices
        .iter()
        .any(|notice| notice.code == "audit_schema_unsupported")
    {
        for (_, agent) in &mut agents_with_start {
            agent.verdict = agent.verdict.worst(AuditVerdict::Unsupported);
        }
    } else if notices.iter().any(|notice| {
        matches!(
            notice.code.as_str(),
            // 被丢弃的观测同样意味着证据不完整：一个 turn_id 不合规而被跳过的 turn
            // 可能正是唯一的 model 违规，不降级就会把它伪装成完整 Match。
            "audit_jsonl_invalid"
                | "audit_line_too_large"
                | "audit_read_error"
                | "audit_bad_identifier"
                | "audit_bad_timestamp"
        )
    }) {
        for (_, agent) in &mut agents_with_start {
            agent.verdict = agent.verdict.worst(AuditVerdict::Incomplete);
        }
    }
    agents_with_start.sort_by(|(left_started, left), (right_started, right)| {
        right_started
            .cmp(left_started)
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let agents = agents_with_start
        .into_iter()
        .map(|(_, agent)| agent)
        .collect();

    SubagentAuditResult {
        schema_version: AUDIT_SCHEMA_VERSION,
        checked_at_ms,
        source_support: if parsed.observations.is_empty() {
            AuditSourceSupport::Unsupported
        } else {
            AuditSourceSupport::RolloutOnly
        },
        agents,
        notices,
    }
}

/// Merge allow-listed outbound sampling observations without exposing the SQLite body.
pub fn merge_outbound_evidence(
    result: &SubagentAuditResult,
    observations: &[OutboundObservation],
) -> SubagentAuditResult {
    let mut updated = result.clone();
    let mut by_turn: BTreeMap<(&str, &str), Vec<&OutboundObservation>> = BTreeMap::new();
    let mut seen = HashSet::new();
    for observation in observations {
        if valid_identifier(&observation.thread_id, MAX_IDENTIFIER_BYTES)
            && valid_identifier(&observation.turn_id, MAX_IDENTIFIER_BYTES)
            && valid_identifier(&observation.requested_model, MAX_IDENTIFIER_BYTES)
        {
            let dedupe_key = (
                observation.thread_id.as_str(),
                observation.turn_id.as_str(),
                observation.requested_model.as_str(),
            );
            if !seen.insert(dedupe_key) {
                continue;
            }
            by_turn
                .entry((&observation.thread_id, &observation.turn_id))
                .or_default()
                .push(observation);
        }
    }

    let mut all_match = !updated.agents.is_empty();
    for agent in &mut updated.agents {
        for turn in &mut agent.turns {
            let key = (agent.thread_id.as_str(), turn.turn_id.as_str());
            let Some(evidence) = by_turn.get(&key) else {
                turn.outbound_verdict = AuditVerdict::Unsupported;
                turn.outbound_requested_model = None;
                turn.outbound_evidence_count = 0;
                turn.verdict = turn.client_verdict.worst(AuditVerdict::Unsupported);
                all_match = false;
                continue;
            };
            let mut models = BTreeSet::new();
            for item in evidence {
                models.insert(item.requested_model.as_str());
            }
            turn.outbound_evidence_count = evidence.len().min(u32::MAX as usize) as u32;
            if models.len() != 1 {
                turn.outbound_verdict = AuditVerdict::Ambiguous;
                turn.outbound_requested_model = None;
            } else {
                let model = *models.iter().next().expect("one model");
                turn.outbound_requested_model = Some(model.to_string());
                let expected = agent.expected_model.as_deref().or(turn.model.as_deref());
                turn.outbound_verdict = match expected {
                    Some(value) if value == model => AuditVerdict::Match,
                    Some(_) => AuditVerdict::Mismatch,
                    None => AuditVerdict::Incomplete,
                };
            }
            turn.verdict = turn.client_verdict.worst(turn.outbound_verdict);
            all_match &= turn.verdict == AuditVerdict::Match;
        }
        // 保留 rollout 归并阶段已施加的降级（截断、未知 schema、JSONL 读取错误、
        // missing/ambiguous）。直接赋值会把它们抹掉，让触达扫描上限的结果显示为完整绿色。
        agent.verdict = agent.verdict.worst(agent.parent.verdict);
        for turn in &agent.turns {
            agent.verdict = agent.verdict.worst(turn.verdict);
        }
        all_match &= !agent.turns.is_empty() && agent.verdict == AuditVerdict::Match;
    }
    if all_match && !matches!(updated.source_support, AuditSourceSupport::Unsupported) {
        updated.source_support = AuditSourceSupport::Full;
    }
    updated
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OutboundObservation {
    pub thread_id: String,
    pub turn_id: String,
    pub requested_model: String,
    pub observed_at_ms: Option<u64>,
}

// ---------- operation audit (pure allow-list and retention helpers) ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OperationAuditPhase {
    Preflight,
    Snapshot,
    Write,
    Verify,
    Completed,
    RolledBack,
    Recovery,
    Audit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OperationAuditResult {
    Success,
    Failed,
    Rejected,
    Busy,
    RolledBack,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationAuditInput {
    pub at_ms: u64,
    pub batch_id: String,
    pub scope: String,
    pub relative_file: Option<String>,
    pub phase: OperationAuditPhase,
    pub result: OperationAuditResult,
    pub error_code: Option<String>,
    pub changed: u32,
    pub unchanged: u32,
    pub files: u32,
    pub role_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OperationAuditRecord {
    pub schema_version: u32,
    pub at_ms: u64,
    pub batch_id: String,
    pub scope: String,
    pub relative_file: Option<String>,
    pub phase: OperationAuditPhase,
    pub result: OperationAuditResult,
    pub error_code: Option<String>,
    pub changed: u32,
    pub unchanged: u32,
    pub files: u32,
    pub role_id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditSanitizeError {
    InvalidBatchId,
    InvalidScope,
}

/// Drop untrusted free-form values at the boundary.  The returned record is the only shape that
/// may be persisted or sent to the UI.
pub fn sanitize_operation_audit(
    input: &OperationAuditInput,
) -> Result<OperationAuditRecord, AuditSanitizeError> {
    if !valid_identifier(&input.batch_id, MAX_IDENTIFIER_BYTES) {
        return Err(AuditSanitizeError::InvalidBatchId);
    }
    if !valid_identifier(&input.scope, MAX_IDENTIFIER_BYTES) {
        return Err(AuditSanitizeError::InvalidScope);
    }
    Ok(OperationAuditRecord {
        schema_version: AUDIT_SCHEMA_VERSION,
        at_ms: input.at_ms,
        batch_id: input.batch_id.clone(),
        scope: input.scope.clone(),
        relative_file: input
            .relative_file
            .as_deref()
            .filter(|value| valid_relative_identifier(value))
            .map(str::to_string),
        phase: input.phase,
        result: input.result,
        error_code: input
            .error_code
            .as_deref()
            .filter(|value| valid_identifier(value, 64))
            .map(str::to_string),
        changed: input.changed,
        unchanged: input.unchanged,
        files: input.files,
        role_id: input
            .role_id
            .as_deref()
            .filter(|value| valid_identifier(value, MAX_IDENTIFIER_BYTES))
            .map(str::to_string),
        model: input
            .model
            .as_deref()
            .filter(|value| valid_identifier(value, MAX_IDENTIFIER_BYTES))
            .map(str::to_string),
        effort: input
            .effort
            .as_deref()
            .filter(|value| valid_identifier(value, MAX_IDENTIFIER_BYTES))
            .map(str::to_string),
    })
}

/// Keep at most 30 days and the newest 500 records.  Input order is retained for the surviving
/// suffix, making repeated pruning deterministic and easy to append atomically.
pub fn retain_operation_audit(
    records: &[OperationAuditRecord],
    now_ms: u64,
) -> Vec<OperationAuditRecord> {
    let mut retained = records
        .iter()
        .filter(|record| {
            record
                .at_ms
                .checked_add(OPERATION_AUDIT_RETENTION_MS)
                .is_some_and(|expiry| expiry >= now_ms)
        })
        .cloned()
        .collect::<Vec<_>>();
    if retained.len() > MAX_OPERATION_AUDIT_ENTRIES {
        let start = retained.len() - MAX_OPERATION_AUDIT_ENTRIES;
        retained.drain(..start);
    }
    retained
}

#[cfg_attr(test, allow(dead_code))]
pub fn operation_audit_jsonl(records: &[OperationAuditRecord]) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    for record in records {
        let line = serde_json::to_vec(&record).map_err(|_| ())?;
        bytes.extend_from_slice(&line);
        bytes.push(b'\n');
    }
    Ok(bytes)
}

// ---------- parser internals ----------

struct JsonlParser<R> {
    reader: R,
    limits: AuditLimits,
    observations: Vec<RolloutObservation>,
    notices: Vec<AuditNotice>,
    truncated: bool,
    current_thread_id: Option<String>,
    line_number: u32,
}

impl<R: Read> JsonlParser<R> {
    fn new(reader: R, limits: AuditLimits) -> Self {
        Self {
            reader,
            limits,
            observations: Vec::new(),
            notices: Vec::new(),
            truncated: false,
            current_thread_id: None,
            line_number: 1,
        }
    }

    fn run(&mut self) {
        let mut chunk = [0u8; 8192];
        let mut line = Vec::new();
        let mut line_too_long = false;
        let mut consumed = 0usize;
        loop {
            if consumed >= self.limits.max_file_bytes {
                if self.limits.max_file_bytes > 0 && consumed == self.limits.max_file_bytes {
                    self.truncated = true;
                    push_notice(&mut self.notices, "audit_truncated");
                }
                let mut probe = [0u8; 1];
                match self.reader.read(&mut probe) {
                    Ok(0) => {}
                    Ok(_) => {
                        self.truncated = true;
                        push_notice(&mut self.notices, "audit_truncated");
                    }
                    Err(_) => push_notice(&mut self.notices, "audit_read_error"),
                }
                break;
            }
            let take = (self.limits.max_file_bytes - consumed).min(chunk.len());
            let read = match self.reader.read(&mut chunk[..take]) {
                Ok(value) => value,
                Err(_) => {
                    push_notice(&mut self.notices, "audit_read_error");
                    break;
                }
            };
            if read == 0 {
                break;
            }
            consumed += read;
            for byte in &chunk[..read] {
                if *byte == b'\n' {
                    if line_too_long {
                        push_notice(&mut self.notices, "audit_line_too_large");
                    } else {
                        self.parse_line(&line);
                    }
                    line.clear();
                    line_too_long = false;
                    self.line_number = self.line_number.saturating_add(1);
                } else if !line_too_long {
                    if line.len() >= self.limits.max_line_bytes {
                        line_too_long = true;
                        line.clear();
                    } else {
                        line.push(*byte);
                    }
                }
            }
        }
        if !line.is_empty() || line_too_long {
            if line_too_long {
                push_notice(&mut self.notices, "audit_line_too_large");
            } else {
                self.parse_line(&line);
            }
        }
    }

    fn parse_line(&mut self, bytes: &[u8]) {
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let value: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => {
                push_notice(&mut self.notices, "audit_jsonl_invalid");
                return;
            }
        };
        let Some(object) = value.as_object() else {
            push_notice(&mut self.notices, "audit_jsonl_invalid");
            return;
        };
        if let Some(version) = object
            .get("schema_version")
            .or_else(|| object.get("schemaVersion"))
        {
            if version.as_u64() != Some(u64::from(AUDIT_SCHEMA_VERSION)) {
                self.current_thread_id = None;
                push_notice(&mut self.notices, "audit_schema_unsupported");
                return;
            }
        }
        let event_type = string_field(object, "type").or_else(|| {
            object
                .get("payload")
                .and_then(Value::as_object)
                .and_then(|p| string_field(p, "type"))
        });
        match event_type {
            Some("session_meta") => self.parse_session(object),
            Some("turn_context") => self.parse_turn(object),
            Some("response_item") => self.parse_response_item(object),
            Some("event_msg") => self.parse_event_msg(object),
            _ => {}
        }
    }

    fn parse_session(&mut self, object: &serde_json::Map<String, Value>) {
        let Some(payload) = object.get("payload").and_then(Value::as_object) else {
            push_notice(&mut self.notices, "audit_incomplete");
            return;
        };
        let id = string_field(payload, "id").or_else(|| string_field(object, "thread_id"));
        let Some(thread_id) = id
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string)
        else {
            push_notice(&mut self.notices, "audit_bad_identifier");
            return;
        };
        self.current_thread_id = Some(thread_id.clone());
        let parent = string_field(payload, "parent_thread_id")
            .or_else(|| nested_string(payload, &["source", "subagent"], "parent_thread_id"))
            .or_else(|| {
                nested_string(
                    payload,
                    &["source", "subagent", "thread_spawn"],
                    "parent_thread_id",
                )
            })
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string);
        let role = string_field(payload, "agent_role")
            .or_else(|| nested_string(payload, &["source", "subagent"], "agent_role"))
            .or_else(|| {
                nested_string(
                    payload,
                    &["source", "subagent", "thread_spawn"],
                    "agent_role",
                )
            })
            .or_else(|| nested_string(payload, &["source", "subagent"], "role"))
            .or_else(|| nested_string(payload, &["source", "subagent", "thread_spawn"], "role"))
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string);
        let path = string_field(payload, "agent_path")
            .or_else(|| nested_string(payload, &["source", "subagent"], "agent_path"))
            .or_else(|| {
                nested_string(
                    payload,
                    &["source", "subagent", "thread_spawn"],
                    "agent_path",
                )
            });
        let join_key = path.and_then(last_path_component).and_then(|value| {
            valid_join_key(value, self.limits.max_identifier_bytes).then(|| value.to_string())
        });
        // The top-level timestamp is the time the rollout line was written.  For session
        // identity the payload timestamp is the actual session start and is stable across
        // repeated session_meta records.
        let event_time = event_time_prefer_payload(object, payload);
        if matches!(event_time, EventTime::Invalid) {
            push_notice(&mut self.notices, "audit_bad_timestamp");
        }
        self.observations
            .push(RolloutObservation::Session(SessionObservation {
                thread_id,
                parent_thread_id: parent,
                agent_role: role,
                started_at_ms: time_value(event_time),
                join_key,
                time_state: time_state(event_time),
                activity_start: false,
            }));
    }

    fn parse_event_msg(&mut self, object: &serde_json::Map<String, Value>) {
        let Some(payload) = object.get("payload").and_then(Value::as_object) else {
            push_notice(&mut self.notices, "audit_incomplete");
            return;
        };
        if string_field(payload, "type") != Some("sub_agent_activity")
            || string_field(payload, "kind") != Some("started")
        {
            return;
        }
        let Some(parent_thread_id) = self.current_thread_id.clone() else {
            push_notice(&mut self.notices, "audit_missing_parent");
            return;
        };
        let Some(thread_id) = string_field(payload, "agent_thread_id")
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string)
        else {
            push_notice(&mut self.notices, "audit_bad_identifier");
            return;
        };
        let path = string_field(payload, "agent_path");
        let join_key = path.and_then(last_path_component).and_then(|value| {
            valid_join_key(value, self.limits.max_identifier_bytes).then(|| value.to_string())
        });
        if path.is_some() && join_key.is_none() {
            push_notice(&mut self.notices, "audit_bad_identifier");
        }
        let event_time = payload
            .get("occurred_at_ms")
            .map_or(EventTime::Missing, parse_event_time);
        if matches!(event_time, EventTime::Invalid) {
            push_notice(&mut self.notices, "audit_bad_timestamp");
        }
        self.observations
            .push(RolloutObservation::Session(SessionObservation {
                thread_id,
                parent_thread_id: Some(parent_thread_id),
                agent_role: None,
                started_at_ms: time_value(event_time),
                join_key,
                time_state: time_state(event_time),
                activity_start: true,
            }));
    }

    fn parse_turn(&mut self, object: &serde_json::Map<String, Value>) {
        let Some(payload) = object.get("payload").and_then(Value::as_object) else {
            push_notice(&mut self.notices, "audit_incomplete");
            return;
        };
        let Some(thread_id) = string_field(object, "thread_id")
            .or_else(|| string_field(payload, "thread_id"))
            .or(self.current_thread_id.as_deref())
            .filter(|value| valid_join_key(value, self.limits.max_identifier_bytes))
            .map(str::to_string)
        else {
            push_notice(&mut self.notices, "audit_bad_identifier");
            return;
        };
        let Some(turn_id) = string_field(payload, "turn_id")
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string)
        else {
            push_notice(&mut self.notices, "audit_bad_identifier");
            return;
        };
        let model = optional_text(payload.get("model"), self.limits.max_identifier_bytes);
        let effort = optional_text(payload.get("effort"), self.limits.max_identifier_bytes);
        let version = optional_text(
            payload.get("multi_agent_version"),
            self.limits.max_identifier_bytes,
        );
        let event_time = event_time(object, payload);
        if matches!(event_time, EventTime::Invalid) {
            push_notice(&mut self.notices, "audit_bad_timestamp");
        }
        self.observations
            .push(RolloutObservation::Turn(TurnObservation {
                thread_id,
                turn_id,
                model: model.value,
                effort: effort.value,
                multi_agent_version: version.value,
                started_at_ms: time_value(event_time),
                time_state: time_state(event_time),
            }));
    }

    fn parse_response_item(&mut self, object: &serde_json::Map<String, Value>) {
        let payload = object
            .get("payload")
            .and_then(Value::as_object)
            .unwrap_or(object);
        let is_spawn = string_field(payload, "name") == Some("spawn_agent")
            || string_field(payload, "tool_name") == Some("spawn_agent")
            || string_field(payload, "action") == Some("spawn_agent");
        if !is_spawn {
            return;
        }
        let Some(parent_thread_id) = string_field(object, "thread_id")
            .or_else(|| string_field(payload, "thread_id"))
            .or(self.current_thread_id.as_deref())
            .filter(|value| valid_identifier(value, self.limits.max_identifier_bytes))
            .map(str::to_string)
        else {
            push_notice(&mut self.notices, "audit_missing_parent");
            return;
        };
        let arguments = match payload.get("arguments") {
            Some(Value::Object(value)) => Some(value.clone()),
            Some(Value::String(value)) => match serde_json::from_str::<Value>(value) {
                Ok(Value::Object(value)) => Some(value),
                _ => {
                    push_notice(&mut self.notices, "audit_ambiguous");
                    None
                }
            },
            _ => {
                push_notice(&mut self.notices, "audit_incomplete");
                None
            }
        };
        let Some(arguments) = arguments else {
            return;
        };
        let task = optional_join_text(arguments.get("task_name"), self.limits.max_identifier_bytes);
        let agent = optional_text(
            arguments.get("agent_type"),
            self.limits.max_identifier_bytes,
        );
        let fork = optional_text(
            arguments.get("fork_turns"),
            self.limits.max_identifier_bytes,
        );
        let model = optional_text(arguments.get("model"), self.limits.max_identifier_bytes);
        let effort = optional_text(
            arguments.get("reasoning_effort"),
            self.limits.max_identifier_bytes,
        );
        let event_time = event_time(object, payload);
        if matches!(event_time, EventTime::Invalid) {
            push_notice(&mut self.notices, "audit_bad_timestamp");
        }
        let task_key = task
            .value
            .as_deref()
            .filter(|value| valid_join_key(value, self.limits.max_identifier_bytes))
            .map(str::to_string);
        self.observations
            .push(RolloutObservation::Spawn(SpawnObservation {
                parent_thread_id,
                task_name_present: task.present,
                observed_agent_type: agent.value,
                observed_fork_turns: fork.value,
                override_model: model.value,
                override_effort: effort.value,
                spawned_at_ms: time_value(event_time),
                task_key,
                task_key_valid: task.valid,
                agent_type_present: agent.present,
                fork_turns_present: fork.present,
                model_present: model.present,
                effort_present: effort.present,
                fields_valid: task.valid
                    && agent.valid
                    && fork.valid
                    && model.valid
                    && effort.valid,
                time_state: time_state(event_time),
            }));
    }

    fn finish(self) -> ParsedRollout {
        ParsedRollout {
            observations: self.observations,
            notices: self.notices,
            truncated: self.truncated,
        }
    }
}

#[derive(Clone)]
struct ChildEvidence {
    thread_id: String,
    session: Option<SessionObservation>,
    session_conflict: bool,
    turns: BTreeMap<String, Vec<TurnObservation>>,
    turn_conflict: HashSet<String>,
}

impl ChildEvidence {
    fn new(thread_id: String) -> Self {
        Self {
            thread_id,
            session: None,
            session_conflict: false,
            turns: BTreeMap::new(),
            turn_conflict: HashSet::new(),
        }
    }
}

fn build_agent_audit(
    child: &ChildEvidence,
    spawns: &BTreeMap<(String, String), Vec<SpawnObservation>>,
    quantity_conflicts: &HashSet<(String, String)>,
    roles: &BTreeMap<String, ManagedRoleSnapshot>,
    invalid_policy_roles: &HashSet<String>,
    notices: &mut Vec<AuditNotice>,
    max_pair_delay_ms: u64,
) -> AgentAudit {
    let session = child.session.as_ref();
    let role_id = session
        .and_then(|value| value.agent_role.clone())
        .unwrap_or_default();
    let role = roles.get(&role_id);
    let role_supported = role.is_some_and(|value| value.managed);
    let (expected_revision, expected_hash, expected_model, expected_effort) = if role_supported {
        role.map(|value| {
            (
                value.policy_revision,
                value.policy_hash.clone(),
                value.expected_model.clone(),
                value.expected_effort.clone(),
            )
        })
        .unwrap_or((None, None, None, None))
    } else {
        (None, None, None, None)
    };
    if role.is_none() || !role_supported {
        push_notice(notices, "audit_role_unmanaged");
    }
    let stale_policy = role
        .and_then(|value| value.policy_updated_at_ms)
        .zip(session.and_then(|value| value.started_at_ms))
        .is_some_and(|(updated, started)| updated > started);
    if stale_policy {
        push_notice(notices, "audit_policy_stale");
    }
    let invalid_policy = invalid_policy_roles.contains(&role_id);
    if invalid_policy {
        push_notice(notices, "audit_policy_invalid");
    }

    let parent = build_parent_audit(
        child,
        spawns,
        quantity_conflicts,
        role_id.as_str(),
        max_pair_delay_ms,
        notices,
    );
    let mut turns = Vec::new();
    for (turn_id, values) in &child.turns {
        let conflict = child.turn_conflict.contains(turn_id) || values.len() != 1;
        let value = values.first();
        let (model, effort, version, timestamp) = if conflict {
            (None, None, None, None)
        } else {
            (
                value.and_then(|item| item.model.clone()),
                value.and_then(|item| item.effort.clone()),
                value.and_then(|item| item.multi_agent_version.clone()),
                value.and_then(|item| item.started_at_ms),
            )
        };
        let client_verdict = if conflict {
            AuditVerdict::Ambiguous
        } else if !role_supported {
            AuditVerdict::Unsupported
        } else if stale_policy || invalid_policy {
            AuditVerdict::Incomplete
        } else if value.is_some_and(|item| matches!(item.time_state, TimeState::Invalid)) {
            AuditVerdict::Ambiguous
        } else if model.is_none() || effort.is_none() || version.is_none() {
            AuditVerdict::Incomplete
        } else if version.as_deref() != Some("v2") {
            AuditVerdict::Unsupported
        } else if expected_model.is_none() || expected_effort.is_none() {
            AuditVerdict::Incomplete
        } else if model != expected_model || effort != expected_effort {
            AuditVerdict::Mismatch
        } else {
            AuditVerdict::Match
        };
        let outbound_verdict = AuditVerdict::Unsupported;
        turns.push(TurnAudit {
            turn_id: turn_id.clone(),
            started_at_ms: timestamp,
            model,
            effort,
            multi_agent_version: version,
            client_verdict,
            outbound_requested_model: None,
            outbound_evidence_count: 0,
            outbound_verdict,
            verdict: client_verdict.worst(outbound_verdict),
        });
    }
    turns.sort_by(compare_turns);
    let mut verdict = parent.verdict;
    for turn in &turns {
        verdict = verdict.worst(turn.verdict);
    }
    if turns.is_empty() {
        push_notice(notices, "audit_missing_turn");
        verdict = verdict.worst(AuditVerdict::Missing);
    }
    if child.session_conflict {
        verdict = verdict.worst(AuditVerdict::Ambiguous);
    }

    AgentAudit {
        thread_id: child.thread_id.clone(),
        role_id,
        expected_policy_revision: expected_revision,
        expected_policy_hash: expected_hash,
        expected_model,
        expected_effort,
        parent,
        turns,
        verdict,
    }
}

fn build_parent_audit(
    child: &ChildEvidence,
    spawns: &BTreeMap<(String, String), Vec<SpawnObservation>>,
    quantity_conflicts: &HashSet<(String, String)>,
    expected_role: &str,
    max_pair_delay_ms: u64,
    notices: &mut Vec<AuditNotice>,
) -> ParentDispatchAudit {
    let Some(session) = child.session.as_ref() else {
        push_notice(notices, "audit_missing_parent");
        return empty_parent(AuditVerdict::Missing);
    };
    let (Some(parent_id), Some(task_key)) =
        (session.parent_thread_id.as_ref(), session.join_key.as_ref())
    else {
        push_notice(notices, "audit_missing_parent");
        return empty_parent(AuditVerdict::Missing);
    };
    let key = (parent_id.clone(), task_key.clone());
    let Some(candidates) = spawns.get(&key) else {
        push_notice(notices, "audit_missing_spawn");
        return empty_parent(AuditVerdict::Missing);
    };
    if quantity_conflicts.contains(&key) || child.session_conflict {
        return empty_parent(AuditVerdict::Ambiguous);
    }
    let Some(spawn) = unique_spawn_candidate(candidates, session, max_pair_delay_ms) else {
        push_notice(notices, "audit_ambiguous");
        return empty_parent(AuditVerdict::Ambiguous);
    };
    let mut verdict = AuditVerdict::Match;
    if !spawn.fields_valid {
        verdict = verdict.worst(AuditVerdict::Ambiguous);
    }
    if !spawn.agent_type_present || spawn.observed_agent_type.is_none() || expected_role.is_empty()
    {
        verdict = verdict.worst(AuditVerdict::Incomplete);
    } else if spawn.observed_agent_type.as_deref() != Some(expected_role) {
        verdict = verdict.worst(AuditVerdict::Mismatch);
    }
    if !spawn.fork_turns_present || spawn.observed_fork_turns.is_none() {
        verdict = verdict.worst(AuditVerdict::Incomplete);
    } else if spawn.observed_fork_turns.as_deref() != Some("none") {
        verdict = verdict.worst(AuditVerdict::Mismatch);
    }
    if spawn.fields_valid {
        if spawn.model_present {
            verdict = verdict.worst(AuditVerdict::Mismatch);
        }
        if spawn.effort_present {
            verdict = verdict.worst(AuditVerdict::Mismatch);
        }
    }
    ParentDispatchAudit {
        verdict,
        observed_agent_type: spawn.observed_agent_type.clone(),
        observed_fork_turns: spawn.observed_fork_turns.clone(),
        override_model: spawn.override_model.clone(),
        override_effort: spawn.override_effort.clone(),
    }
}

fn unique_spawn_candidate<'a>(
    candidates: &'a [SpawnObservation],
    session: &SessionObservation,
    max_pair_delay_ms: u64,
) -> Option<&'a SpawnObservation> {
    if candidates.len() != 1 {
        return None;
    }
    let candidate = &candidates[0];
    let (TimeState::Valid(spawned), TimeState::Valid(started)) =
        (candidate.time_state, session.time_state)
    else {
        return None;
    };
    if spawned > started || started - spawned > max_pair_delay_ms {
        return None;
    }
    Some(candidate)
}

fn empty_parent(verdict: AuditVerdict) -> ParentDispatchAudit {
    ParentDispatchAudit {
        verdict,
        observed_agent_type: None,
        observed_fork_turns: None,
        override_model: None,
        override_effort: None,
    }
}

fn compare_turns(left: &TurnAudit, right: &TurnAudit) -> Ordering {
    match (left.started_at_ms, right.started_at_ms) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| left.turn_id.cmp(&right.turn_id))
}

/// Combine the parent-side activity marker with the child-side session metadata.  These records
/// describe the same child but intentionally carry different timestamps and role completeness;
/// a conflicting identity (parent/path/role) remains ambiguous.
fn merge_session_observation(
    previous: &SessionObservation,
    incoming: &SessionObservation,
) -> Option<SessionObservation> {
    if previous.thread_id != incoming.thread_id
        || (previous.parent_thread_id.is_some()
            && incoming.parent_thread_id.is_some()
            && previous.parent_thread_id != incoming.parent_thread_id)
        || (previous.join_key.is_some()
            && incoming.join_key.is_some()
            && previous.join_key != incoming.join_key)
        || (previous.agent_role.is_some()
            && incoming.agent_role.is_some()
            && previous.agent_role != incoming.agent_role)
    {
        return None;
    }
    if previous.activity_start
        && incoming.activity_start
        && (previous.started_at_ms != incoming.started_at_ms
            || previous.time_state != incoming.time_state)
    {
        return None;
    }
    if !previous.activity_start
        && !incoming.activity_start
        && (previous.started_at_ms != incoming.started_at_ms
            || previous.time_state != incoming.time_state)
    {
        return None;
    }

    let mut merged = previous.clone();
    merged.parent_thread_id = previous
        .parent_thread_id
        .clone()
        .or_else(|| incoming.parent_thread_id.clone());
    merged.agent_role = previous
        .agent_role
        .clone()
        .or_else(|| incoming.agent_role.clone());
    merged.join_key = previous
        .join_key
        .clone()
        .or_else(|| incoming.join_key.clone());

    // Activity time is the only timestamp that can be paired with the parent spawn.  Preserve it
    // even when the child's session_meta timestamp is a few milliseconds earlier.
    if incoming.activity_start && !previous.activity_start {
        merged.started_at_ms = incoming.started_at_ms;
        merged.time_state = incoming.time_state;
        merged.activity_start = true;
    }
    Some(merged)
}

fn turn_equivalent(left: &TurnObservation, right: &TurnObservation) -> bool {
    left.thread_id == right.thread_id
        && left.turn_id == right.turn_id
        && left.model == right.model
        && left.effort == right.effort
        && left.multi_agent_version == right.multi_agent_version
        && left.started_at_ms == right.started_at_ms
        && left.time_state == right.time_state
}

fn push_notice(notices: &mut Vec<AuditNotice>, code: &str) {
    if !notices.iter().any(|notice| notice.code == code) {
        notices.push(AuditNotice {
            code: code.to_string(),
        });
    }
}

fn valid_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
        && !value.starts_with('/')
        && !value.contains("..")
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_join_key(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value
            .bytes()
            .all(|byte| (0x20..=0x7e).contains(&byte) && !matches!(byte, b'/' | b'\\'))
}

fn valid_relative_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.bytes().any(|byte| byte.is_ascii_control())
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
}

fn string_field<'a>(object: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn nested_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    path: &[&str],
    key: &str,
) -> Option<&'a str> {
    let mut current = object;
    for component in path {
        current = current.get(*component)?.as_object()?;
    }
    string_field(current, key)
}

fn optional_text(value: Option<&Value>, max_bytes: usize) -> TextField {
    let Some(value) = value else {
        return TextField {
            present: false,
            value: None,
            valid: true,
        };
    };
    match value {
        Value::Null => TextField {
            // JSON null is the serialized form used by the runtime for an omitted optional
            // override.  It must not turn an absent model/effort override into a mismatch.
            present: false,
            value: None,
            valid: true,
        },
        Value::String(value) => TextField {
            present: true,
            value: valid_identifier(value, max_bytes).then(|| value.clone()),
            valid: valid_identifier(value, max_bytes),
        },
        _ => TextField {
            present: true,
            value: None,
            valid: false,
        },
    }
}

fn optional_join_text(value: Option<&Value>, max_bytes: usize) -> TextField {
    let Some(value) = value else {
        return TextField {
            present: false,
            value: None,
            valid: true,
        };
    };
    match value {
        Value::Null => TextField {
            present: false,
            value: None,
            valid: true,
        },
        Value::String(value) => TextField {
            present: true,
            value: valid_join_key(value, max_bytes).then(|| value.clone()),
            valid: valid_join_key(value, max_bytes),
        },
        _ => TextField {
            present: true,
            value: None,
            valid: false,
        },
    }
}

fn event_time(
    object: &serde_json::Map<String, Value>,
    payload: &serde_json::Map<String, Value>,
) -> EventTime {
    for key in [
        "timestamp_ms",
        "created_at_ms",
        "event_time_ms",
        "timestamp",
    ] {
        if let Some(value) = object.get(key).or_else(|| payload.get(key)) {
            return parse_event_time(value);
        }
    }
    EventTime::Missing
}

fn event_time_prefer_payload(
    object: &serde_json::Map<String, Value>,
    payload: &serde_json::Map<String, Value>,
) -> EventTime {
    for key in [
        "timestamp_ms",
        "created_at_ms",
        "event_time_ms",
        "timestamp",
    ] {
        if let Some(value) = payload.get(key).or_else(|| object.get(key)) {
            return parse_event_time(value);
        }
    }
    EventTime::Missing
}

fn parse_event_time(value: &Value) -> EventTime {
    match value {
        Value::Number(number) => number
            .as_u64()
            .and_then(scale_timestamp)
            .map_or(EventTime::Invalid, EventTime::Valid),
        Value::String(value) => {
            parse_timestamp_string(value).map_or(EventTime::Invalid, EventTime::Valid)
        }
        _ => EventTime::Invalid,
    }
}

fn scale_timestamp(value: u64) -> Option<u64> {
    if value >= 10_000_000_000 {
        Some(value)
    } else {
        value.checked_mul(1_000)
    }
}

fn parse_timestamp_string(value: &str) -> Option<u64> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse::<u64>().ok().and_then(scale_timestamp);
    }
    parse_rfc3339_ms(value)
}

fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let separator = bytes.get(10).copied()?;
    if separator != b'T' && separator != b't' && separator != b' ' {
        return None;
    }
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    let year = parse_digits(&bytes[0..4])? as i64;
    let month = parse_digits(&bytes[5..7])? as u32;
    let day = parse_digits(&bytes[8..10])? as u32;
    let hour = parse_digits(&bytes[11..13])?;
    let minute = parse_digits(&bytes[14..16])?;
    let second = parse_digits(&bytes[17..19])?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut index = 19usize;
    let mut millis = 0u64;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if start == index {
            return None;
        }
        let digits = (index - start).min(3);
        millis = parse_digits(&bytes[start..start + digits])? * 10u64.pow((3 - digits) as u32);
    }
    let offset_minutes = match bytes.get(index).copied() {
        Some(b'Z') | Some(b'z') if index + 1 == bytes.len() => 0i64,
        Some(sign @ (b'+' | b'-')) => {
            if index + 6 != bytes.len() || bytes.get(index + 3) != Some(&b':') {
                return None;
            }
            let hours = parse_digits(&bytes[index + 1..index + 3])? as i64;
            let minutes = parse_digits(&bytes[index + 4..index + 6])? as i64;
            if hours > 23 || minutes > 59 {
                return None;
            }
            let value = hours * 60 + minutes;
            if sign == b'-' {
                -value
            } else {
                value
            }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour as i64 * 3_600 + minute as i64 * 60 + second as i64)?
        .checked_sub(offset_minutes * 60)?;
    if seconds < 0 {
        return None;
    }
    (seconds as u64)
        .checked_mul(1_000)
        .and_then(|value| value.checked_add(millis))
}

fn parse_digits(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0u64, |value, byte| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(*byte - b'0')))
    })
}

// Howard Hinnant's civil-date conversion, kept local to avoid a time dependency.
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era.checked_mul(146_097)
        .and_then(|value| value.checked_add(day_of_era))
        .and_then(|value| value.checked_sub(719_468))
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn last_path_component(path: &str) -> Option<&str> {
    path.split(['/', '\\']).rfind(|part| !part.is_empty())
}

fn time_value(time: EventTime) -> Option<u64> {
    match time {
        EventTime::Valid(value) => Some(value),
        EventTime::Missing | EventTime::Invalid => None,
    }
}

fn time_state(time: EventTime) -> TimeState {
    match time {
        EventTime::Missing => TimeState::Missing,
        EventTime::Valid(value) => TimeState::Valid(value),
        EventTime::Invalid => TimeState::Invalid,
    }
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> AuditLimits {
        AuditLimits {
            max_date_directories: 7,
            max_rollout_files: 500,
            max_file_bytes: 16 * 1024 * 1024,
            max_line_bytes: 2 * 1024 * 1024,
            max_children: 20,
            max_pair_delay_ms: 120_000,
            max_identifier_bytes: 128,
        }
    }

    fn child_fixture(extra: &str) -> String {
        format!(
            "{{\"type\":\"session_meta\",\"timestamp_ms\":1000,\"payload\":{{\"id\":\"child-1\",\"parent_thread_id\":\"parent-1\",\"agent_path\":\"root/worker\",\"agent_role\":\"worker\"}}}}\n{{\"type\":\"turn_context\",\"thread_id\":\"child-1\",\"timestamp_ms\":1100,\"payload\":{{\"turn_id\":\"turn-1\",\"model\":\"gpt-test\",\"effort\":\"high\",\"multi_agent_version\":\"v2\"}}}}\n{extra}"
        )
    }

    fn role() -> ManagedRoleSnapshot {
        ManagedRoleSnapshot::new("worker", "gpt-test", "high")
    }

    #[test]
    fn checked_fixture_is_sanitized_and_deterministic() {
        let parsed = parse_rollout_bytes(
            include_bytes!("fixtures/audit/rollout-base.jsonl"),
            limits(),
        );
        let result = merge_rollout_evidence(&parsed, &[role()], 2_000);
        assert_eq!(result.schema_version, AUDIT_SCHEMA_VERSION);
        assert_eq!(result.agents.len(), 1);
        assert!(!serde_json::to_string(&result)
            .expect("serialize audit DTO")
            .contains("ignored"));
    }

    #[test]
    fn bounded_parser_extracts_allowlisted_events_without_message() {
        let input = child_fixture(
            "{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":900,\"payload\":{\"name\":\"spawn_agent\",\"arguments\":\"{\\\"task_name\\\":\\\"worker\\\",\\\"agent_type\\\":\\\"worker\\\",\\\"fork_turns\\\":\\\"none\\\"}\",\"message\":\"secret prompt\"}}",
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        assert_eq!(parsed.observations.len(), 3);
        let debug = format!("{:?}", parsed.observations[0]);
        assert!(!debug.contains("secret prompt"));
        let result = merge_rollout_evidence(&parsed, &[role()], 2000);
        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].parent.verdict, AuditVerdict::Match);
        assert_eq!(
            result.agents[0].turns[0].client_verdict,
            AuditVerdict::Match
        );
        assert_eq!(result.agents[0].verdict, AuditVerdict::Unsupported);
    }

    #[test]
    fn subagent_activity_enriches_child_session_and_filters_root() {
        let input = concat!(
            "{\"type\":\"session_meta\",\"timestamp_ms\":1000,\"payload\":{\"id\":\"parent-1\",\"timestamp_ms\":1000}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"sub_agent_activity\",\"kind\":\"started\",\"occurred_at_ms\":1050,\"agent_thread_id\":\"child-1\",\"agent_path\":\"/root/worker\"}}\n",
            "{\"type\":\"session_meta\",\"timestamp_ms\":1100,\"payload\":{\"id\":\"child-1\",\"parent_thread_id\":\"parent-1\",\"timestamp_ms\":1040,\"agent_path\":\"/root/worker\",\"agent_role\":\"worker\"}}\n",
            "{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":1000,\"payload\":{\"name\":\"spawn_agent\",\"arguments\":{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\",\"model\":null,\"reasoning_effort\":null}}}\n",
            "{\"type\":\"turn_context\",\"thread_id\":\"child-1\",\"timestamp_ms\":1060,\"payload\":{\"turn_id\":\"turn-1\",\"model\":\"gpt-test\",\"effort\":\"high\",\"multi_agent_version\":\"v2\"}}"
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        assert_eq!(
            parsed
                .observations
                .iter()
                .filter(|observation| matches!(observation, RolloutObservation::Session(_)))
                .count(),
            3
        );
        let result = merge_rollout_evidence(&parsed, &[role()], 2_000);
        assert_eq!(result.agents.len(), 1);
        let agent = &result.agents[0];
        assert_eq!(agent.thread_id, "child-1");
        assert_eq!(agent.role_id, "worker");
        assert_eq!(agent.parent.verdict, AuditVerdict::Match);
        assert_eq!(agent.turns[0].client_verdict, AuditVerdict::Match);
        assert_eq!(agent.turns[0].started_at_ms, Some(1_060_000));
        assert_eq!(agent.verdict, AuditVerdict::Unsupported);
        assert!(!result
            .notices
            .iter()
            .any(|notice| notice.code == "audit_ambiguous"));
    }

    #[test]
    fn duplicate_conflicting_turn_context_is_ambiguous() {
        let input = format!(
            "{}\n{{\"type\":\"turn_context\",\"thread_id\":\"child-1\",\"timestamp_ms\":1100,\"payload\":{{\"turn_id\":\"turn-1\",\"model\":\"other\",\"effort\":\"high\",\"multi_agent_version\":\"v2\"}}}}",
            child_fixture("{}")
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        let result = merge_rollout_evidence(&parsed, &[role()], 2000);
        assert_eq!(result.agents[0].turns[0].verdict, AuditVerdict::Ambiguous);
        assert_eq!(result.agents[0].verdict, AuditVerdict::Ambiguous);
    }

    #[test]
    fn duplicate_spawn_candidates_do_not_choose_latest() {
        let input = format!(
            "{}\n{{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":900,\"payload\":{{\"name\":\"spawn_agent\",\"arguments\":{{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\"}}}}}}",
            child_fixture("{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":800,\"payload\":{\"name\":\"spawn_agent\",\"arguments\":{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\"}}}")
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        let result = merge_rollout_evidence(&parsed, &[role()], 2000);
        assert_eq!(result.agents[0].parent.verdict, AuditVerdict::Ambiguous);
    }

    #[test]
    fn truncation_downgrades_a_matching_agent() {
        let input = child_fixture("{}");
        let mut small = limits();
        small.max_file_bytes = input.len() - 1;
        let parsed = parse_rollout_bytes(input.as_bytes(), small);
        assert!(parsed.truncated);
        let result = merge_rollout_evidence(&parsed, &[role()], 2000);
        assert_ne!(result.agents[0].verdict, AuditVerdict::Match);
        assert!(result
            .notices
            .iter()
            .any(|notice| notice.code == "audit_truncated"));
    }

    /// The outbound merge runs after the rollout merge and must not reset the verdict it
    /// computed. Overwriting it discards every truncation/unsupported downgrade, so a
    /// scan that hit its limits reports a fully green `Match` with a `truncated` notice.
    #[test]
    fn truncation_survives_the_outbound_merge() {
        // Every layer of evidence matches; only the child budget is exhausted.
        let input = concat!(
            "{\"type\":\"session_meta\",\"timestamp_ms\":1000,\"payload\":{\"id\":\"parent-1\",\"timestamp_ms\":1000}}\n",
            "{\"type\":\"session_meta\",\"timestamp_ms\":1100,\"payload\":{\"id\":\"child-1\",\"parent_thread_id\":\"parent-1\",\"timestamp_ms\":1040,\"agent_path\":\"/root/worker\",\"agent_role\":\"worker\"}}\n",
            "{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":1000,\"payload\":{\"name\":\"spawn_agent\",\"arguments\":{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\",\"model\":null,\"reasoning_effort\":null}}}\n",
            "{\"type\":\"turn_context\",\"thread_id\":\"child-1\",\"timestamp_ms\":1060,\"payload\":{\"turn_id\":\"turn-1\",\"model\":\"gpt-test\",\"effort\":\"high\",\"multi_agent_version\":\"v2\"}}"
        );
        let mut capped = limits();
        capped.max_children = 1;
        let parsed = parse_rollout_bytes(input.as_bytes(), capped);
        let rollout = merge_rollout_evidence_with_limits(&parsed, &[role()], 2000, capped);
        assert_eq!(
            rollout.agents[0].parent.verdict,
            AuditVerdict::Match,
            "parent evidence itself must be clean so only the truncation downgrade is under test"
        );
        assert!(
            rollout
                .notices
                .iter()
                .any(|notice| notice.code == "audit_truncated"),
            "reaching the child budget must record a truncation notice"
        );
        assert_ne!(
            rollout.agents[0].verdict,
            AuditVerdict::Match,
            "hitting the child budget must downgrade the agent"
        );

        let observations = rollout.agents[0]
            .turns
            .iter()
            .map(|turn| OutboundObservation {
                thread_id: rollout.agents[0].thread_id.clone(),
                turn_id: turn.turn_id.clone(),
                requested_model: turn.model.clone().unwrap_or_else(|| "gpt-test".to_string()),
                observed_at_ms: None,
            })
            .collect::<Vec<_>>();
        let merged = merge_outbound_evidence(&rollout, &observations);

        assert_ne!(
            merged.agents[0].verdict,
            AuditVerdict::Match,
            "a truncated scan must never end up fully green"
        );
        assert!(
            !matches!(merged.source_support, AuditSourceSupport::Full),
            "truncated evidence must not claim full source support"
        );
        assert!(merged
            .notices
            .iter()
            .any(|notice| notice.code == "audit_truncated"));
    }

    #[test]
    fn operation_audit_drops_invalid_relative_paths_and_keeps_retention_boundary() {
        let input = OperationAuditInput {
            at_ms: 1,
            batch_id: "batch-1".to_string(),
            scope: "global".to_string(),
            relative_file: Some("/private/prompt.txt".to_string()),
            phase: OperationAuditPhase::Completed,
            result: OperationAuditResult::Success,
            error_code: Some("ok".to_string()),
            changed: 1,
            unchanged: 0,
            files: 1,
            role_id: Some("worker".to_string()),
            model: Some("gpt-test".to_string()),
            effort: Some("high".to_string()),
        };
        let record = sanitize_operation_audit(&input).unwrap();
        assert_eq!(record.relative_file, None);
        assert!(!serde_json::to_string(&record).unwrap().contains("private"));
        let boundary = record.at_ms + OPERATION_AUDIT_RETENTION_MS;
        assert_eq!(retain_operation_audit(&[record], boundary).len(), 1);
    }

    #[test]
    fn invalid_timestamp_is_not_paired() {
        let input = child_fixture(
            "{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":\"bad\",\"payload\":{\"name\":\"spawn_agent\",\"arguments\":{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\"}}}",
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        let result = merge_rollout_evidence(&parsed, &[role()], 2000);
        assert_eq!(result.agents[0].parent.verdict, AuditVerdict::Ambiguous);
    }

    #[test]
    fn unknown_schema_is_not_treated_as_success() {
        let parsed = parse_rollout_bytes(
            br#"{"schemaVersion":99,"type":"session_meta","payload":{"id":"child-1"}}"#,
            limits(),
        );
        assert!(parsed.observations.is_empty());
        assert!(parsed
            .notices
            .iter()
            .any(|notice| notice.code == "audit_schema_unsupported"));
    }

    #[test]
    fn unknown_schema_notice_downgrades_otherwise_valid_agent() {
        let valid = child_fixture(
            "{\"type\":\"response_item\",\"thread_id\":\"parent-1\",\"timestamp_ms\":900,\"payload\":{\"name\":\"spawn_agent\",\"arguments\":{\"task_name\":\"worker\",\"agent_type\":\"worker\",\"fork_turns\":\"none\"}}}",
        );
        let input = format!(
            "{}\n{{\"schemaVersion\":99,\"type\":\"unrelated\",\"payload\":{{}}}}",
            valid
        );
        let parsed = parse_rollout_bytes(input.as_bytes(), limits());
        let result = merge_rollout_evidence(&parsed, &[role()], 2_000);
        assert_eq!(result.agents[0].verdict, AuditVerdict::Unsupported);
    }

    #[test]
    fn outbound_duplicate_rows_are_deduplicated_and_conflicts_are_visible() {
        let mut result = SubagentAuditResult {
            schema_version: AUDIT_SCHEMA_VERSION,
            checked_at_ms: 1,
            source_support: AuditSourceSupport::RolloutOnly,
            agents: vec![AgentAudit {
                thread_id: "child-1".to_string(),
                role_id: "worker".to_string(),
                expected_policy_revision: None,
                expected_policy_hash: None,
                expected_model: Some("gpt-test".to_string()),
                expected_effort: Some("high".to_string()),
                parent: empty_parent(AuditVerdict::Match),
                turns: vec![TurnAudit {
                    turn_id: "turn-1".to_string(),
                    started_at_ms: Some(2),
                    model: Some("gpt-test".to_string()),
                    effort: Some("high".to_string()),
                    multi_agent_version: Some("v2".to_string()),
                    client_verdict: AuditVerdict::Match,
                    outbound_requested_model: None,
                    outbound_evidence_count: 0,
                    outbound_verdict: AuditVerdict::Unsupported,
                    verdict: AuditVerdict::Unsupported,
                }],
                verdict: AuditVerdict::Unsupported,
            }],
            notices: Vec::new(),
        };
        let observations = vec![
            OutboundObservation {
                thread_id: "child-1".to_string(),
                turn_id: "turn-1".to_string(),
                requested_model: "gpt-test".to_string(),
                observed_at_ms: Some(99),
            },
            OutboundObservation {
                thread_id: "child-1".to_string(),
                turn_id: "turn-1".to_string(),
                requested_model: "gpt-test".to_string(),
                observed_at_ms: Some(3),
            },
            OutboundObservation {
                thread_id: "child-1".to_string(),
                turn_id: "turn-1".to_string(),
                requested_model: "other".to_string(),
                observed_at_ms: Some(4),
            },
        ];
        result = merge_outbound_evidence(&result, &observations);
        assert_eq!(result.agents[0].turns[0].outbound_evidence_count, 2);
        assert_eq!(
            result.agents[0].turns[0].outbound_verdict,
            AuditVerdict::Ambiguous
        );
    }

    #[test]
    fn exact_file_budget_is_marked_truncated() {
        let input = b"{}\n";
        let mut budget = limits();
        budget.max_file_bytes = input.len();
        let parsed = parse_rollout_bytes(input, budget);
        assert!(parsed.truncated);
        assert!(parsed
            .notices
            .iter()
            .any(|notice| notice.code == "audit_truncated"));
    }

    #[test]
    fn direct_jsonl_reader_marks_exact_file_budget_truncated() {
        let input = b"{}\n";
        let mut budget = limits();
        budget.max_file_bytes = input.len();
        let parsed = parse_rollout_jsonl(std::io::Cursor::new(input), budget);
        assert!(parsed.truncated);
        assert!(parsed
            .notices
            .iter()
            .any(|notice| notice.code == "audit_truncated"));
    }
}
