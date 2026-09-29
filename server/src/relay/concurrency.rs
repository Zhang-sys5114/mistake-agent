//! 每用户并发闸门（ADR-0047 决策 9）。
//!
//! 进程内计数：单实例单上游，够用且零依赖。
//! **若将来水平扩展，这里必须挪到共享存储**（ADR-0047 修订 R9 已预先记下这条）。
//!
//! 名额用 `Drop` 归还：无论请求正常结束、上游报错还是任务被取消，都不会漏还名额
//! （漏还的后果是该用户永久无法发起请求，比多放一次严重得多）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use uuid::Uuid;

#[derive(Default)]
struct Inner {
    in_flight: HashMap<Uuid, i32>,
}

/// 并发闸门（可克隆，内部共享）。
#[derive(Clone, Default)]
pub struct ConcurrencyGate {
    inner: Arc<Mutex<Inner>>,
}

/// 名额凭据：离开作用域即归还。
pub struct GateGuard {
    gate: ConcurrencyGate,
    user_id: Uuid,
}

impl ConcurrencyGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// 占用一个名额；已达上限返回 `None`（调用方映射为 429）。
    pub fn acquire(&self, user_id: Uuid, limit: i32) -> Option<GateGuard> {
        let limit = limit.max(1);
        let mut inner = self.lock();
        let count = inner.in_flight.entry(user_id).or_insert(0);
        if *count >= limit {
            return None;
        }
        *count += 1;
        Some(GateGuard {
            gate: self.clone(),
            user_id,
        })
    }

    /// 当前在飞请求数（测试与诊断用）。
    pub fn in_flight(&self, user_id: Uuid) -> i32 {
        self.lock().in_flight.get(&user_id).copied().unwrap_or(0)
    }

    /// 锁被毒化（某次 panic）时继续用内部数据：这个计数只用于限流，
    /// 没有"必须保守失败"的语义，而 panic 会变成用户看到的 500。
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Drop for GateGuard {
    fn drop(&mut self) {
        let mut inner = self.gate.lock();
        if let Some(count) = inner.in_flight.get_mut(&self.user_id) {
            *count -= 1;
            // 归零即移除，避免 HashMap 随用户数无限增长
            if *count <= 0 {
                inner.in_flight.remove(&self.user_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_is_enforced_and_released_on_drop() {
        let gate = ConcurrencyGate::new();
        let user = Uuid::new_v4();

        let first = gate.acquire(user, 2).expect("第 1 个名额");
        let second = gate.acquire(user, 2).expect("第 2 个名额");
        assert!(gate.acquire(user, 2).is_none(), "第 3 个应被拒绝");
        assert_eq!(gate.in_flight(user), 2);

        drop(first);
        assert_eq!(gate.in_flight(user), 1);
        let _third = gate.acquire(user, 2).expect("归还后应能再占用");

        drop(second);
        drop(_third);
        assert_eq!(gate.in_flight(user), 0);
    }

    #[test]
    fn users_do_not_interfere() {
        let gate = ConcurrencyGate::new();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let _held = gate.acquire(a, 1).expect("A 占满自己的名额");
        assert!(gate.acquire(a, 1).is_none(), "A 自己应被限");
        assert!(gate.acquire(b, 1).is_some(), "B 不该受影响");
    }

    #[test]
    fn zero_limit_is_treated_as_one() {
        // 配错成 0 时不能让用户完全无法使用
        let gate = ConcurrencyGate::new();
        let user = Uuid::new_v4();
        let held = gate.acquire(user, 0);
        assert!(held.is_some());
        assert!(gate.acquire(user, 0).is_none());
    }
}
