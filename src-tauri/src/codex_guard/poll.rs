//! 轮询（60s，仅 launcher 运行期间）：锁定参数漂移即自动恢复（写入前备份），
//! 恢复记录落盘，不弹通知（CONTEXT.md 语义边界）

use crate::config;

use super::engine::{apply, check, expected_of};
use super::files::{builtin_files, load_files};
use crate::codex_fs::now_secs;
use super::schema::load_schema;

pub async fn poll_loop() {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Err(e) = poll_once() {
            log::error!("codex guard 轮询失败: {}", e);
        }
    }
}

fn poll_once() -> Result<(), String> {
    let mut cfg = config::load_config()?;
    if !cfg.codex_guard.enabled {
        return Ok(());
    }
    let schema = load_schema();
    // 只看守文件列表内的目标文件，与 UI 可见范围一致（CONTEXT.md：UI 完全由合并结果驱动）
    let files = load_files().unwrap_or_else(|_| builtin_files());
    let mut dirty = false;
    for p in &schema {
        if !files.iter().any(|f| f.file == p.file) {
            continue;
        }
        let locked = cfg
            .codex_guard
            .params
            .get(&p.id)
            .is_some_and(|s| s.locked);
        if !locked {
            continue;
        }
        let expected = expected_of(p, cfg.codex_guard.params.get(&p.id));
        let c = check(p, &expected);
        let st = cfg.codex_guard.params.entry(p.id.clone()).or_default();
        st.last_checked = Some(now_secs());
        dirty = true;
        if c.status == "drift" || c.status == "missing" {
            match apply(p, &expected) {
                Ok(()) => {
                    st.last_restored = Some(now_secs());
                    log::info!("codex guard 已自动恢复: {}", p.id);
                }
                Err(e) => log::error!("codex guard 恢复 {} 失败: {}", p.id, e),
            }
        }
    }
    if dirty {
        config::save_config(&cfg)?;
    }
    Ok(())
}
