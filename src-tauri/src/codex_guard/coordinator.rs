//! Guard 写入口共享协调器。
//!
//! 协调器只负责进程内的非排队互斥与恢复状态；事务 journal、快照和原子替换仍由
//! `engine`/`journal` 负责。调用方必须在执行任何主动写入前拿到 `try_write`。

use serde::Serialize;
use specta::Type;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

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

    pub(crate) fn recovery_status(&self) -> GuardRecoveryStatus {
        self.recovery
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| GuardRecoveryStatus {
                blocked: true,
                code: Some("recovery_state_unavailable".to_string()),
            })
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
    fn poll_start_claim_is_idempotent() {
        let coordinator = GuardCoordinator::new();
        assert!(coordinator.claim_poll_start());
        assert!(!coordinator.claim_poll_start());
    }
}
