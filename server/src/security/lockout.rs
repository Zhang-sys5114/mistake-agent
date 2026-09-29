//! 失败封禁（fail2ban 语义，进程内版）。
//!
//! 规则：在 `window` 内累计 `threshold` 次失败 → 封禁 `block` 时长；
//! 封禁期内一律拒绝（调用方返回 429 + `Retry-After`）；成功一次即清零。
//!
//! 与真正的 fail2ban 的关系：本模块负责"应用层立刻止血"（不依赖外部进程立即生效），
//! 同时把失败事件写成**固定格式的日志行**（`AUTH_FAIL ip=... reason=...`），
//! 供外部 fail2ban 匹配并做更长期的 IP 级封禁（配置见 `server/deploy/fail2ban/`）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const DEFAULT_KEYS_CAP: usize = 50_000;

#[derive(Debug, Clone, Copy)]
struct FailureState {
    count: u32,
    first_at: Instant,
    blocked_until: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    states: HashMap<String, FailureState>,
}

/// 失败计数器 + 临时封禁（可克隆，内部共享）。
#[derive(Clone)]
pub struct FailureGuard {
    inner: Arc<Mutex<Inner>>,
    threshold: u32,
    window: Duration,
    block: Duration,
    keys_cap: usize,
}

impl FailureGuard {
    pub fn new(threshold: u32, window: Duration, block: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            threshold: threshold.max(1),
            window,
            block,
            keys_cap: DEFAULT_KEYS_CAP,
        }
    }

    /// 该 key 当前是否被禁。`Err(retry_after)` 表示还在封禁期内。
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut inner = self.lock();
        let Some(state) = inner.states.get_mut(key) else {
            return Ok(());
        };
        if let Some(until) = state.blocked_until {
            if until > now {
                return Err(until.duration_since(now));
            }
            // 封禁到期：清零重来（不清的话下次失败会立刻又被封）
            inner.states.remove(key);
        } else if now.duration_since(state.first_at) >= self.window {
            // 计数窗口已过，视为从未失败
            inner.states.remove(key);
        }
        Ok(())
    }

    /// 记一次失败；达到阈值即开始封禁。返回是否刚触发封禁（供日志使用）。
    pub fn record_failure(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.states.len() >= self.keys_cap {
            inner
                .states
                .retain(|_, state| state.blocked_until.is_some_and(|until| until > now));
        }

        let state = inner.states.entry(key.to_string()).or_insert(FailureState {
            count: 0,
            first_at: now,
            blocked_until: None,
        });
        if now.duration_since(state.first_at) >= self.window {
            state.count = 0;
            state.first_at = now;
        }
        state.count += 1;
        if state.count >= self.threshold {
            state.blocked_until = Some(now + self.block);
            state.count = 0;
            state.first_at = now;
            return true;
        }
        false
    }

    /// 成功一次即清零（避免正常用户被自己偶尔的输错拖进封禁）。
    pub fn record_success(&self, key: &str) {
        self.lock().states.remove(key);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_threshold_failures() {
        let guard = FailureGuard::new(3, Duration::from_secs(60), Duration::from_secs(30));
        assert!(guard.check("ip").is_ok());
        assert!(!guard.record_failure("ip"));
        assert!(!guard.record_failure("ip"));
        assert!(guard.record_failure("ip"), "第 3 次应触发封禁");
        let retry_after = guard.check("ip").expect_err("封禁期内应被拒");
        assert!(retry_after <= Duration::from_secs(30));
    }

    #[test]
    fn success_clears_the_counter() {
        let guard = FailureGuard::new(3, Duration::from_secs(60), Duration::from_secs(30));
        guard.record_failure("k");
        guard.record_failure("k");
        guard.record_success("k");
        assert!(!guard.record_failure("k"), "清零后不该立刻触发封禁");
        assert!(guard.check("k").is_ok());
    }

    #[test]
    fn failure_window_expires() {
        let guard = FailureGuard::new(2, Duration::from_millis(30), Duration::from_secs(30));
        guard.record_failure("k");
        std::thread::sleep(Duration::from_millis(50));
        assert!(!guard.record_failure("k"), "窗口过后应重新计数");
        assert!(guard.check("k").is_ok());
    }

    #[test]
    fn block_expires_and_resets() {
        let guard = FailureGuard::new(1, Duration::from_secs(60), Duration::from_millis(30));
        assert!(guard.record_failure("k"));
        assert!(guard.check("k").is_err());
        std::thread::sleep(Duration::from_millis(50));
        assert!(guard.check("k").is_ok(), "封禁到期后应放行");
    }

    #[test]
    fn keys_are_independent() {
        let guard = FailureGuard::new(1, Duration::from_secs(60), Duration::from_secs(30));
        guard.record_failure("a");
        assert!(guard.check("a").is_err());
        assert!(guard.check("b").is_ok());
    }
}
