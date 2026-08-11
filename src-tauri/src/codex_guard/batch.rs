//! 批量生命周期操作的纯领域模型。
//!
//! 该模块只解析作用域、聚合成员状态并计算动作矩阵/eligibility；文件计划、配置保存
//! 和事务执行仍由现有 engine/coordinator 负责。

use serde::{Deserialize, Serialize};

use super::lifecycle::{HealthStatus, ParameterLifecycle};
use super::model::ValidationDiagnostic;

#[cfg(test)]
use super::lifecycle::{aggregate_health, aggregate_lifecycle, LifecycleSummary};
#[cfg(test)]
use std::borrow::Borrow;

pub const BATCH_CONTRACT_SCHEMA_VERSION: u32 = 1;

/// Stable lifecycle stages emitted while a batch is executing.  The payload deliberately
/// contains no path, file contents, or parameter values; consumers can only render progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum GuardOperationPhase {
    Preflight,
    Snapshot,
    Write,
    Verify,
    Completed,
    Recovery,
}

#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, tauri_specta::Event,
)]
#[serde(rename_all = "camelCase")]
pub struct GuardOperationProgress {
    pub batch_id: String,
    pub phase: GuardOperationPhase,
    pub completed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchPreview {
    pub schema_version: u32,
    pub preview_id: String,
    /// SHA-256 revision of the semantic Launcher configuration used for the preview.
    pub revision: String,
    pub scope: BatchScope,
    pub action: BatchAction,
    pub member_ids: Vec<String>,
    pub affected_members: u32,
    pub affected_files: u32,
    pub changed: u32,
    pub unchanged: u32,
    pub files: u32,
    pub eligible: bool,
    pub blockers: Vec<ValidationDiagnostic>,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

/// 批量命令作用域。Role 由托管角色记录展开为角色文件和角色目录成员。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchScope {
    All,
    Group {
        #[serde(rename = "groupId")]
        group_id: String,
    },
    Parameter {
        #[serde(rename = "parameterId")]
        parameter_id: String,
    },
    Role {
        #[serde(rename = "roleId")]
        role_id: String,
    },
}

impl BatchScope {
    #[cfg(test)]
    pub const fn all() -> Self {
        Self::All
    }

    #[cfg(test)]
    pub fn group(group_id: impl Into<String>) -> Self {
        Self::Group {
            group_id: group_id.into(),
        }
    }

    #[cfg(test)]
    pub fn parameter(parameter_id: impl Into<String>) -> Self {
        Self::Parameter {
            parameter_id: parameter_id.into(),
        }
    }

    #[cfg(test)]
    pub fn role(role_id: impl Into<String>) -> Self {
        Self::Role {
            role_id: role_id.into(),
        }
    }
}

/// 批量生命周期动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchAction {
    Apply,
    Lock,
    Unlock,
    Disable,
}

impl BatchAction {
    pub const fn writes_target_file(self) -> bool {
        matches!(self, Self::Apply | Self::Lock)
    }
}

/// 批量请求合同。`schema_version` 是持久/IPC 边界的显式版本，不接受隐式默认。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchRequest {
    pub schema_version: u32,
    pub scope: BatchScope,
    pub action: BatchAction,
}

impl BatchRequest {
    #[cfg(test)]
    pub fn new(scope: BatchScope, action: BatchAction) -> Self {
        Self {
            schema_version: BATCH_CONTRACT_SCHEMA_VERSION,
            scope,
            action,
        }
    }

    pub const fn is_supported_version(&self) -> bool {
        self.schema_version == BATCH_CONTRACT_SCHEMA_VERSION
    }
}

/// 批量事务的最终结果；业务操作不返回部分成功。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchOutcome {
    Committed,
    Rejected,
    RolledBack,
    CriticalRecovery,
}

impl BatchOutcome {}

/// 批量事务报告。诊断使用 Guard 统一的稳定 code，不携带解析器原文。
///
/// 当前 `ValidationDiagnostic` 仍是内部类型，所以报告暂不派生 `specta::Type`；接入公开
/// command 合同时由合同层统一提升诊断 DTO 的可见性。
#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchReport {
    pub schema_version: u32,
    pub batch_id: String,
    pub outcome: BatchOutcome,
    pub changed: u32,
    pub unchanged: u32,
    pub files: u32,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl BatchReport {
    #[cfg(test)]
    pub fn new(batch_id: impl Into<String>, outcome: BatchOutcome) -> Self {
        Self {
            schema_version: BATCH_CONTRACT_SCHEMA_VERSION,
            batch_id: batch_id.into(),
            outcome,
            changed: 0,
            unchanged: 0,
            files: 0,
            diagnostics: Vec::new(),
        }
    }
}

/// 作用域成员引用。字段保持字符串以兼容 schema/migration 层，解析结果按 id 稳定排序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchMember {
    pub id: String,
    pub group_id: Option<String>,
    pub role_id: Option<String>,
    pub lifecycle: ParameterLifecycle,
    pub health: HealthStatus,
}

impl BatchMember {
    pub fn new(id: impl Into<String>, lifecycle: ParameterLifecycle, health: HealthStatus) -> Self {
        Self {
            id: id.into(),
            group_id: None,
            role_id: None,
            lifecycle,
            health,
        }
    }

    pub fn in_group(mut self, group_id: impl Into<String>) -> Self {
        self.group_id = Some(group_id.into());
        self
    }

    pub fn for_role(mut self, role_id: impl Into<String>) -> Self {
        self.role_id = Some(role_id.into());
        self
    }
}

/// Apply/Lock 前的阻塞原因。字符串值是稳定的前端翻译 code。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum BatchEligibilityReason {
    Invalid,
    Unsupported,
    Error,
    StateUnavailable,
    RecoveryInProgress,
    OwnershipUncertain,
}

impl BatchEligibilityReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Error => "error",
            Self::StateUnavailable => "state_unavailable",
            Self::RecoveryInProgress => "recovery_in_progress",
            Self::OwnershipUncertain => "ownership_uncertain",
        }
    }
}

/// 单个成员的动作可执行性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchEligibility {
    pub eligible: bool,
    pub reason: Option<BatchEligibilityReason>,
}

impl BatchEligibility {
    pub const fn allowed() -> Self {
        Self {
            eligible: true,
            reason: None,
        }
    }

    pub const fn blocked(reason: BatchEligibilityReason) -> Self {
        Self {
            eligible: false,
            reason: Some(reason),
        }
    }
}

/// 计算 eligibility 时的边界状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EligibilityContext {
    pub lifecycle: ParameterLifecycle,
    pub health: HealthStatus,
    /// Launcher lifecycle 是否可读；不可读时四种动作都必须拒绝。
    pub state_readable: bool,
    /// 是否存在未完成事务恢复；恢复期间拒绝用户批量写入。
    pub transaction_recovering: bool,
    /// 文件所有权预检是否确定；不确定时四种动作都必须拒绝。
    pub ownership_known: bool,
}

impl EligibilityContext {
    pub const fn new(lifecycle: ParameterLifecycle, health: HealthStatus) -> Self {
        Self {
            lifecycle,
            health,
            state_readable: true,
            transaction_recovering: false,
            ownership_known: true,
        }
    }
}

/// 动作后的成员计划。`writes_file` 表示该成员会参与 Codex 文件写计划。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberActionPlan {
    pub lifecycle: ParameterLifecycle,
    pub changed: bool,
    pub writes_file: bool,
}

/// 根据动作矩阵计算一个成员的目标状态与文件写入影响。
pub fn plan_member_action(
    action: BatchAction,
    lifecycle: ParameterLifecycle,
    health: HealthStatus,
) -> MemberActionPlan {
    let target = match action {
        BatchAction::Apply => match lifecycle {
            ParameterLifecycle::Disabled => ParameterLifecycle::Applied,
            ParameterLifecycle::Applied | ParameterLifecycle::Locked => lifecycle,
        },
        BatchAction::Lock => match lifecycle {
            ParameterLifecycle::Disabled | ParameterLifecycle::Applied => {
                ParameterLifecycle::Locked
            }
            ParameterLifecycle::Locked => ParameterLifecycle::Locked,
        },
        BatchAction::Unlock => match lifecycle {
            ParameterLifecycle::Locked => ParameterLifecycle::Applied,
            ParameterLifecycle::Disabled | ParameterLifecycle::Applied => lifecycle,
        },
        BatchAction::Disable => ParameterLifecycle::Disabled,
    };
    let writes_file = match action {
        BatchAction::Apply => match lifecycle {
            ParameterLifecycle::Disabled => true,
            ParameterLifecycle::Applied => matches!(health, HealthStatus::Drifted),
            ParameterLifecycle::Locked => false,
        },
        BatchAction::Lock => match lifecycle {
            ParameterLifecycle::Disabled => true,
            ParameterLifecycle::Applied => matches!(health, HealthStatus::Drifted),
            ParameterLifecycle::Locked => false,
        },
        BatchAction::Unlock | BatchAction::Disable => false,
    };
    MemberActionPlan {
        lifecycle: target,
        changed: target != lifecycle || writes_file,
        writes_file,
    }
}

/// 计算单成员 eligibility。
pub const fn evaluate_eligibility(
    action: BatchAction,
    context: EligibilityContext,
) -> BatchEligibility {
    if !context.state_readable {
        return BatchEligibility::blocked(BatchEligibilityReason::StateUnavailable);
    }
    if context.transaction_recovering {
        return BatchEligibility::blocked(BatchEligibilityReason::RecoveryInProgress);
    }
    if !context.ownership_known {
        return BatchEligibility::blocked(BatchEligibilityReason::OwnershipUncertain);
    }
    if matches!(action, BatchAction::Apply | BatchAction::Lock) {
        let reason = match context.health {
            HealthStatus::Invalid => Some(BatchEligibilityReason::Invalid),
            HealthStatus::Unsupported => Some(BatchEligibilityReason::Unsupported),
            HealthStatus::Error => Some(BatchEligibilityReason::Error),
            HealthStatus::Healthy | HealthStatus::Drifted => None,
        };
        if let Some(reason) = reason {
            return BatchEligibility::blocked(reason);
        }
    }
    BatchEligibility::allowed()
}

/// 常用的简化入口：边界状态均视为可读、非恢复、所有权确定。
#[cfg(test)]
pub const fn is_action_eligible(
    action: BatchAction,
    lifecycle: ParameterLifecycle,
    health: HealthStatus,
) -> bool {
    evaluate_eligibility(action, EligibilityContext::new(lifecycle, health)).eligible
}

/// 简化入口的语义别名。
#[cfg(test)]
pub const fn eligibility_for(
    action: BatchAction,
    lifecycle: ParameterLifecycle,
    health: HealthStatus,
) -> BatchEligibility {
    evaluate_eligibility(action, EligibilityContext::new(lifecycle, health))
}

/// 判断成员是否属于作用域。
pub fn scope_matches(scope: &BatchScope, member: &BatchMember) -> bool {
    match scope {
        BatchScope::All => true,
        BatchScope::Group { group_id } => member.group_id.as_deref() == Some(group_id.as_str()),
        BatchScope::Parameter { parameter_id } => member.id == *parameter_id,
        BatchScope::Role { role_id } => member.role_id.as_deref() == Some(role_id.as_str()),
    }
}

/// 解析作用域并按稳定参数 ID 排序，避免输入 schema 顺序影响 journal/报告。
pub fn members_in_scope<'a>(
    scope: &BatchScope,
    members: &'a [BatchMember],
) -> Vec<&'a BatchMember> {
    let mut selected = members
        .iter()
        .filter(|member| scope_matches(scope, member))
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.id.cmp(&right.id));
    selected
}

/// 批量成员状态聚合；生命周期和健康状态都从成员派生，不保存第二份组状态。
#[cfg(test)]
pub fn aggregate_member_status<I, T>(members: I) -> (LifecycleSummary, HealthStatus)
where
    I: IntoIterator<Item = T>,
    T: Borrow<BatchMember>,
{
    let members = members.into_iter().collect::<Vec<_>>();
    (
        aggregate_lifecycle(members.iter().map(|member| &member.borrow().lifecycle)),
        aggregate_health(members.iter().map(|member| &member.borrow().health)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(
        id: &str,
        group: &str,
        role: &str,
        lifecycle: ParameterLifecycle,
        health: HealthStatus,
    ) -> BatchMember {
        BatchMember::new(id, lifecycle, health)
            .in_group(group)
            .for_role(role)
    }

    #[test]
    fn action_matrix_covers_all_four_actions_and_three_lifecycles() {
        let cases = [
            (
                BatchAction::Apply,
                ParameterLifecycle::Disabled,
                ParameterLifecycle::Applied,
                true,
                true,
            ),
            (
                BatchAction::Apply,
                ParameterLifecycle::Applied,
                ParameterLifecycle::Applied,
                false,
                false,
            ),
            (
                BatchAction::Apply,
                ParameterLifecycle::Locked,
                ParameterLifecycle::Locked,
                false,
                false,
            ),
            (
                BatchAction::Lock,
                ParameterLifecycle::Disabled,
                ParameterLifecycle::Locked,
                true,
                true,
            ),
            (
                BatchAction::Lock,
                ParameterLifecycle::Applied,
                ParameterLifecycle::Locked,
                true,
                false,
            ),
            (
                BatchAction::Lock,
                ParameterLifecycle::Locked,
                ParameterLifecycle::Locked,
                false,
                false,
            ),
            (
                BatchAction::Unlock,
                ParameterLifecycle::Disabled,
                ParameterLifecycle::Disabled,
                false,
                false,
            ),
            (
                BatchAction::Unlock,
                ParameterLifecycle::Applied,
                ParameterLifecycle::Applied,
                false,
                false,
            ),
            (
                BatchAction::Unlock,
                ParameterLifecycle::Locked,
                ParameterLifecycle::Applied,
                true,
                false,
            ),
            (
                BatchAction::Disable,
                ParameterLifecycle::Disabled,
                ParameterLifecycle::Disabled,
                false,
                false,
            ),
            (
                BatchAction::Disable,
                ParameterLifecycle::Applied,
                ParameterLifecycle::Disabled,
                true,
                false,
            ),
            (
                BatchAction::Disable,
                ParameterLifecycle::Locked,
                ParameterLifecycle::Disabled,
                true,
                false,
            ),
        ];
        for (action, initial, target, changed, writes_file) in cases {
            let plan = plan_member_action(action, initial, HealthStatus::Healthy);
            assert_eq!(plan.lifecycle, target, "{action:?} {initial:?}");
            assert_eq!(plan.changed, changed, "{action:?} {initial:?}");
            assert_eq!(plan.writes_file, writes_file, "{action:?} {initial:?}");
        }
        let drifted_apply = plan_member_action(
            BatchAction::Apply,
            ParameterLifecycle::Applied,
            HealthStatus::Drifted,
        );
        assert!(drifted_apply.changed);
        assert!(drifted_apply.writes_file);
        assert!(
            plan_member_action(
                BatchAction::Lock,
                ParameterLifecycle::Applied,
                HealthStatus::Drifted
            )
            .writes_file
        );
    }

    #[test]
    fn health_eligibility_blocks_apply_and_lock_but_allows_unlock_and_disable() {
        for health in [
            HealthStatus::Invalid,
            HealthStatus::Unsupported,
            HealthStatus::Error,
        ] {
            for action in [BatchAction::Apply, BatchAction::Lock] {
                let result = eligibility_for(action, ParameterLifecycle::Applied, health);
                assert!(!result.eligible);
                assert!(result.reason.is_some());
            }
            for action in [BatchAction::Unlock, BatchAction::Disable] {
                assert!(is_action_eligible(
                    action,
                    ParameterLifecycle::Locked,
                    health
                ));
            }
        }
    }

    #[test]
    fn boundary_eligibility_blocks_every_action() {
        let mut context =
            EligibilityContext::new(ParameterLifecycle::Locked, HealthStatus::Healthy);
        context.state_readable = false;
        assert_eq!(
            evaluate_eligibility(BatchAction::Disable, context).reason,
            Some(BatchEligibilityReason::StateUnavailable)
        );
        context.state_readable = true;
        context.transaction_recovering = true;
        assert_eq!(
            evaluate_eligibility(BatchAction::Unlock, context).reason,
            Some(BatchEligibilityReason::RecoveryInProgress)
        );
        context.transaction_recovering = false;
        context.ownership_known = false;
        assert_eq!(
            evaluate_eligibility(BatchAction::Apply, context).reason,
            Some(BatchEligibilityReason::OwnershipUncertain)
        );
    }

    #[test]
    fn all_group_parameter_and_role_scopes_are_stable_and_sorted() {
        let members = vec![
            member(
                "z",
                "g1",
                "r1",
                ParameterLifecycle::Applied,
                HealthStatus::Healthy,
            ),
            member(
                "a",
                "g1",
                "r2",
                ParameterLifecycle::Disabled,
                HealthStatus::Healthy,
            ),
            member(
                "m",
                "g2",
                "r1",
                ParameterLifecycle::Locked,
                HealthStatus::Healthy,
            ),
        ];
        let ids = |scope| {
            members_in_scope(&scope, &members)
                .into_iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(BatchScope::All), vec!["a", "m", "z"]);
        assert_eq!(ids(BatchScope::group("g1")), vec!["a", "z"]);
        assert_eq!(ids(BatchScope::parameter("m")), vec!["m"]);
        assert_eq!(ids(BatchScope::role("r1")), vec!["m", "z"]);
        assert!(ids(BatchScope::group("missing")).is_empty());
    }

    #[test]
    fn aggregate_status_does_not_depend_on_scope_storage() {
        let members = vec![
            member(
                "a",
                "g",
                "r",
                ParameterLifecycle::Applied,
                HealthStatus::Drifted,
            ),
            member(
                "b",
                "g",
                "r",
                ParameterLifecycle::Locked,
                HealthStatus::Invalid,
            ),
            member(
                "c",
                "g",
                "r",
                ParameterLifecycle::Locked,
                HealthStatus::Error,
            ),
        ];
        assert_eq!(
            aggregate_member_status(&members),
            (LifecycleSummary::Mixed, HealthStatus::Error)
        );
        assert_eq!(
            aggregate_member_status(Vec::<BatchMember>::new()),
            (LifecycleSummary::Disabled, HealthStatus::Healthy)
        );
    }

    #[test]
    fn request_and_report_use_stable_wire_names() {
        let request = BatchRequest::new(BatchScope::group("subagent"), BatchAction::Lock);
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value["schemaVersion"], BATCH_CONTRACT_SCHEMA_VERSION);
        assert_eq!(value["action"], "lock");
        assert_eq!(value["scope"]["group"]["groupId"], "subagent");

        let report = BatchReport::new("batch-1", BatchOutcome::Committed);
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["outcome"], "committed");
        assert_eq!(value["changed"], 0);
    }
}
