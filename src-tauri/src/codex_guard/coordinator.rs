//! Guard 写入口共享协调器。
//!
//! 协调器只负责进程内的非排队互斥与恢复状态；事务 journal、快照和原子替换仍由
//! `engine`/`journal` 负责。调用方必须在执行任何主动写入前拿到 `try_write`。

use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use super::batch::BatchRequest;
use super::now_secs;

/// 前端可见的恢复状态。只返回稳定 code，不暴露 journal 路径、文件内容或底层错误。
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GuardRecoveryStatus {
    pub blocked: bool,
    pub code: Option<String>,
}

#[derive(Clone)]
pub(crate) struct GuardCoordinator {
    write: Arc<Mutex<()>>,
    recovery: Arc<Mutex<GuardRecoveryStatus>>,
    poll_started: Arc<AtomicBool>,
    previews: Arc<Mutex<HashMap<String, PreviewRecord>>>,
}

struct PreviewRecord {
    request: BatchRequest,
    revision: String,
    expires_at: u64,
}

impl GuardCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            write: Arc::new(Mutex::new(())),
            recovery: Arc::new(Mutex::new(GuardRecoveryStatus {
                blocked: false,
                code: None,
            })),
            poll_started: Arc::new(AtomicBool::new(false)),
            previews: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 用户命令不排队：已有写入时立即返回稳定的 `guard_busy` code。
    pub(crate) fn try_write(&self) -> Result<MutexGuard<'_, ()>, String> {
        match self.write.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err("guard_busy".to_string()),
            Err(TryLockError::Poisoned(_)) => Err("guard_lock_poisoned".to_string()),
        }
    }

    /// 普通主动写入的门：恢复未完成时 fail closed；恢复重试命令使用裸 `try_write`。
    pub(crate) fn try_guard_write(&self) -> Result<MutexGuard<'_, ()>, String> {
        let guard = self.try_write()?;
        if self.recovery_blocked() {
            drop(guard);
            return Err("recovery_blocked".to_string());
        }
        Ok(guard)
    }

    /// 轮询不排队，也不在恢复阻断期间尝试写入。
    pub(crate) fn try_poll_write(&self) -> Result<Option<MutexGuard<'_, ()>>, String> {
        let guard = match self.write.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => return Err("guard_lock_poisoned".to_string()),
        };
        if self.recovery_blocked() {
            drop(guard);
            return Ok(None);
        }
        Ok(Some(guard))
    }

    pub(crate) fn recovery_status(&self) -> GuardRecoveryStatus {
        self.recovery
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| GuardRecoveryStatus {
                blocked: true,
                code: Some("recovery_state_unavailable".to_string()),
            })
    }

    fn recovery_blocked(&self) -> bool {
        self.recovery
            .lock()
            .map(|status| status.blocked)
            .unwrap_or(true)
    }

    pub(crate) fn mark_recovery_blocked(&self, code: &str) {
        if let Ok(mut status) = self.recovery.lock() {
            status.blocked = true;
            status.code = Some(code.to_string());
        }
    }

    pub(crate) fn clear_recovery(&self) {
        if let Ok(mut status) = self.recovery.lock() {
            status.blocked = false;
            status.code = None;
        }
    }

    /// 只允许启动一个轮询任务；恢复重试成功后可安全调用，不会产生重复轮询。
    pub(crate) fn claim_poll_start(&self) -> bool {
        !self.poll_started.swap(true, Ordering::AcqRel)
    }

    pub(crate) fn remember_preview(&self, id: String, request: BatchRequest, revision: String) {
        if let Ok(mut previews) = self.previews.lock() {
            let now = now_secs();
            previews.retain(|_, record| record.expires_at > now);
            previews.insert(
                id,
                PreviewRecord {
                    request,
                    revision,
                    expires_at: now.saturating_add(300),
                },
            );
        }
    }

    pub(crate) fn take_preview(&self, id: &str, request: &BatchRequest, revision: &str) -> bool {
        let Ok(mut previews) = self.previews.lock() else {
            return false;
        };
        let now = now_secs();
        previews.retain(|_, record| record.expires_at > now);
        previews.remove(id).is_some_and(|record| {
            record.expires_at > now && record.request == *request && record.revision == revision
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_write_is_non_queueing_and_uses_stable_busy_code() {
        let coordinator = GuardCoordinator::new();
        let _held = coordinator.try_write().unwrap();
        assert_eq!(coordinator.try_write().unwrap_err(), "guard_busy");
    }

    #[test]
    fn recovery_status_contains_only_stable_state() {
        let coordinator = GuardCoordinator::new();
        assert_eq!(
            coordinator.recovery_status(),
            GuardRecoveryStatus {
                blocked: false,
                code: None,
            }
        );
        coordinator.mark_recovery_blocked("recovery_failed");
        assert_eq!(
            coordinator.recovery_status(),
            GuardRecoveryStatus {
                blocked: true,
                code: Some("recovery_failed".to_string()),
            }
        );
        coordinator.clear_recovery();
        assert!(!coordinator.recovery_status().blocked);
    }

    #[test]
    fn ordinary_writes_fail_closed_until_recovery_is_clear() {
        let coordinator = GuardCoordinator::new();
        coordinator.mark_recovery_blocked("recovery_failed");
        assert_eq!(
            coordinator.try_guard_write().unwrap_err(),
            "recovery_blocked"
        );
        assert!(coordinator.try_poll_write().unwrap().is_none());
        coordinator.clear_recovery();
        assert!(coordinator.try_guard_write().is_ok());
    }

    #[test]
    fn poll_start_claim_is_idempotent() {
        let coordinator = GuardCoordinator::new();
        assert!(coordinator.claim_poll_start());
        assert!(!coordinator.claim_poll_start());
    }

    #[test]
    fn preview_is_invalidated_when_configuration_revision_changes() {
        let coordinator = GuardCoordinator::new();
        let request = BatchRequest::new(
            super::super::batch::BatchScope::all(),
            super::super::batch::BatchAction::Apply,
        );
        coordinator.remember_preview("preview".to_string(), request.clone(), "rev-a".to_string());
        assert!(!coordinator.take_preview("preview", &request, "rev-b"));
        coordinator.remember_preview("preview".to_string(), request.clone(), "rev-a".to_string());
        assert!(coordinator.take_preview("preview", &request, "rev-a"));
    }
}
