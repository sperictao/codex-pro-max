//! Guard 参数生命周期与健康状态的纯领域模型。
//!
//! 生命周期只允许 `Disabled -> Applied -> Locked` 这三个合法值。旧版本把
//! `applied` 和 `locked` 存成两个布尔值，因此 `locked=true, applied=false`
//! 必须在迁移边界显式拒绝，而不能静默猜测。

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

/// 单个参数的合法托管生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ParameterLifecycle {
    Disabled,
    Applied,
    Locked,
}

impl ParameterLifecycle {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Applied => "applied",
            Self::Locked => "locked",
        }
    }

    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Locked)
    }

    /// 将 v0 的两个布尔值转换为合法生命周期。
    ///
    /// `locked=true, applied=false` 没有安全的隐式解释，必须交给迁移向导。
    pub const fn from_flags(applied: bool, locked: bool) -> Result<Self, InvalidLifecycleFlags> {
        match (applied, locked) {
            (false, false) => Ok(Self::Disabled),
            (true, false) => Ok(Self::Applied),
            (true, true) => Ok(Self::Locked),
            (false, true) => Err(InvalidLifecycleFlags { applied, locked }),
        }
    }

    /// 别名，供迁移代码使用更明确的调用点。
    pub const fn from_legacy_flags(
        applied: bool,
        locked: bool,
    ) -> Result<Self, InvalidLifecycleFlags> {
        Self::from_flags(applied, locked)
    }
}

impl TryFrom<(bool, bool)> for ParameterLifecycle {
    type Error = InvalidLifecycleFlags;

    fn try_from(flags: (bool, bool)) -> Result<Self, Self::Error> {
        Self::from_flags(flags.0, flags.1)
    }
}

impl fmt::Display for ParameterLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 旧 v0 状态中无法表示为合法生命周期的布尔组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidLifecycleFlags {
    pub applied: bool,
    pub locked: bool,
}

impl InvalidLifecycleFlags {
    #[cfg(test)]
    #[allow(dead_code)]
    pub const fn code(self) -> &'static str {
        "invalid_lifecycle"
    }
}

impl fmt::Display for InvalidLifecycleFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid lifecycle flags: applied={}, locked={}",
            self.applied, self.locked
        )
    }
}

impl std::error::Error for InvalidLifecycleFlags {}

/// 组或全局成员生命周期的派生摘要。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleSummary {
    #[default]
    Disabled,
    Applied,
    Locked,
    Mixed,
}

impl LifecycleSummary {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Applied => "applied",
            Self::Locked => "locked",
            Self::Mixed => "mixed",
        }
    }
}

impl fmt::Display for LifecycleSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 健康状态独立于生命周期，数值越大优先级越高。
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    #[default]
    Healthy,
    Drifted,
    Invalid,
    Unsupported,
    Error,
}

impl HealthStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Drifted => "drifted",
            Self::Invalid => "invalid",
            Self::Unsupported => "unsupported",
            Self::Error => "error",
        }
    }

    /// 聚合优先级：Error > Invalid > Unsupported > Drifted > Healthy。
    pub const fn priority(self) -> u8 {
        match self {
            Self::Healthy => 0,
            Self::Drifted => 1,
            Self::Unsupported => 2,
            Self::Invalid => 3,
            Self::Error => 4,
        }
    }
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 从成员状态派生生命周期摘要。空集合按“没有启用成员”处理为 Disabled。
pub fn summarize_lifecycle<I, T>(members: I) -> LifecycleSummary
where
    I: IntoIterator<Item = T>,
    T: Borrow<ParameterLifecycle>,
{
    let mut iter = members.into_iter();
    let Some(first) = iter.next() else {
        return LifecycleSummary::Disabled;
    };
    let first = *first.borrow();
    if iter.all(|member| *member.borrow() == first) {
        return match first {
            ParameterLifecycle::Disabled => LifecycleSummary::Disabled,
            ParameterLifecycle::Applied => LifecycleSummary::Applied,
            ParameterLifecycle::Locked => LifecycleSummary::Locked,
        };
    }
    LifecycleSummary::Mixed
}

/// `summarize_lifecycle` 的语义别名，便于调用点表达聚合意图。
pub fn aggregate_lifecycle<I, T>(members: I) -> LifecycleSummary
where
    I: IntoIterator<Item = T>,
    T: Borrow<ParameterLifecycle>,
{
    summarize_lifecycle(members)
}

/// 从成员健康状态派生健康摘要。空集合没有阻塞状态，按 Healthy 处理。
pub fn summarize_health<I, T>(members: I) -> HealthStatus
where
    I: IntoIterator<Item = T>,
    T: Borrow<HealthStatus>,
{
    members
        .into_iter()
        .map(|member| *member.borrow())
        .max_by_key(|status| status.priority())
        .unwrap_or(HealthStatus::Healthy)
}

/// `summarize_health` 的语义别名。
pub fn aggregate_health<I, T>(members: I) -> HealthStatus
where
    I: IntoIterator<Item = T>,
    T: Borrow<HealthStatus>,
{
    summarize_health(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_flags_accept_only_the_three_valid_states() {
        assert_eq!(
            ParameterLifecycle::from_flags(false, false),
            Ok(ParameterLifecycle::Disabled)
        );
        assert_eq!(
            ParameterLifecycle::from_flags(true, false),
            Ok(ParameterLifecycle::Applied)
        );
        assert_eq!(
            ParameterLifecycle::from_flags(true, true),
            Ok(ParameterLifecycle::Locked)
        );
    }

    #[test]
    fn locked_without_applied_is_an_explicit_migration_blocker() {
        let error = ParameterLifecycle::from_flags(false, true).unwrap_err();
        assert_eq!(
            error,
            InvalidLifecycleFlags {
                applied: false,
                locked: true
            }
        );
        assert_eq!(error.code(), "invalid_lifecycle");
    }

    #[test]
    fn summaries_use_mixed_and_health_priority() {
        assert_eq!(
            summarize_lifecycle(Vec::<ParameterLifecycle>::new()),
            LifecycleSummary::Disabled
        );
        assert_eq!(
            summarize_lifecycle([ParameterLifecycle::Applied, ParameterLifecycle::Applied]),
            LifecycleSummary::Applied
        );
        assert_eq!(
            summarize_lifecycle([ParameterLifecycle::Disabled, ParameterLifecycle::Locked]),
            LifecycleSummary::Mixed
        );
        assert_eq!(
            summarize_health([
                HealthStatus::Healthy,
                HealthStatus::Drifted,
                HealthStatus::Unsupported,
                HealthStatus::Invalid,
                HealthStatus::Error,
            ]),
            HealthStatus::Error
        );
        assert_eq!(
            summarize_health([HealthStatus::Healthy, HealthStatus::Drifted]),
            HealthStatus::Drifted
        );
    }

    #[test]
    fn summaries_accept_borrowed_members() {
        let lifecycle = vec![ParameterLifecycle::Locked, ParameterLifecycle::Locked];
        let health = vec![HealthStatus::Healthy, HealthStatus::Invalid];
        assert_eq!(summarize_lifecycle(&lifecycle), LifecycleSummary::Locked);
        assert_eq!(summarize_health(&health), HealthStatus::Invalid);
    }
}
