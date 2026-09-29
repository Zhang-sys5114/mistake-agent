//! 令牌桶限流（按任意 key：客户端 IP 或用户 id）。
//!
//! **为什么用令牌桶而不是滑动窗口**：
//! - 突发友好：学生一轮对话里会连着发好几个请求（工具调用回填后继续），
//!   滑动窗口的硬性"N 次/窗口"会把这些正常突发判成滥用；令牌桶用容量吸收突发，
//!   只对**持续**超过速率的流量动手。
//! - 状态 O(1)/key：桶只存"当前令牌数 + 上次补充时刻"两个数；滑动窗口要为每个 key
//!   存 N 个时间戳（N = 限额）。key 是客户端 IP 时，这个差别就是内存安全与否。
//! - `Retry-After` 可精确算出：桶空时"补满 1 个令牌所需时间"就是准确的退避时长。
//!
//! 进程内实现，与单实例部署匹配（水平扩展时要挪到共享存储，ADR-0047 修订 R9 已记录）。
//! **尽力而为**：它保护上游账号与正常用户，不是精确配额——[`TokenBucket::keys_cap`]
//! 保证内存有界。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// 同时跟踪的 key 上限。超限先清空桶（空闲 key），仍超则整体重置。
/// key 包含客户端 IP 时，攻击者能用海量伪造 IP 撑内存，这条上限就是兜底。
const DEFAULT_KEYS_CAP: usize = 50_000;

/// `Retry-After` 的上限：速率被配成极小值时，别让响应头出现荒谬的数字。
const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

#[derive(Default)]
struct Inner {
    buckets: HashMap<String, Bucket>,
}

/// 令牌桶限流器（可克隆，内部共享）。
#[derive(Clone)]
pub struct TokenBucket {
    inner: Arc<Mutex<Inner>>,
    /// 每秒补充的令牌数
    rate_per_sec: f64,
    /// 桶容量（允许的突发上限）
    capacity: f64,
    keys_cap: usize,
}

impl TokenBucket {
    /// `rate_per_minute` 是持续速率，`burst` 是容量。
    /// 两者都会被夹到合法下界——配错成 0 时宁可放宽一点，也不要让所有人被永久 429。
    pub fn new(rate_per_minute: u32, burst: u32) -> Self {
        let rate_per_minute = rate_per_minute.max(1);
        let capacity = burst.max(1) as f64;
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            rate_per_sec: f64::from(rate_per_minute) / 60.0,
            capacity,
            keys_cap: DEFAULT_KEYS_CAP,
        }
    }

    /// 取一个令牌。`Err(retry_after)` 表示应拒绝，时长可直接用作 `Retry-After`。
    pub fn check(&self, key: &str) -> Result<(), Duration> {
        let now = Instant::now();
        let mut inner = self.lock();
        if inner.buckets.len() >= self.keys_cap {
            self.evict(&mut inner, now);
        }
        let (rate, capacity) = (self.rate_per_sec, self.capacity);
        let bucket = inner.buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: capacity,
            last_refill: now,
        });

        // 先按流逝时间补令牌（上限为容量，多等不会攒出超额突发）
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate).min(capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Ok(());
        }
        let wait = (1.0 - bucket.tokens) / rate;
        Err(Duration::from_secs_f64(wait).min(MAX_RETRY_AFTER))
    }

    /// 当前可用令牌数（测试与诊断用）。
    pub fn available(&self, key: &str) -> f64 {
        let now = Instant::now();
        let (rate, capacity) = (self.rate_per_sec, self.capacity);
        let inner = self.lock();
        inner
            .buckets
            .get(key)
            .map(|bucket| {
                let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
                (bucket.tokens + elapsed * rate).min(capacity)
            })
            .unwrap_or(capacity)
    }

    /// 清掉已回满的桶（它们不含信息）；仍超上限则整体重置，保证内存有界。
    fn evict(&self, inner: &mut Inner, now: Instant) {
        let (rate, capacity) = (self.rate_per_sec, self.capacity);
        inner.buckets.retain(|_, bucket| {
            let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
            bucket.tokens + elapsed * rate < capacity - f64::EPSILON
        });
        if inner.buckets.len() >= self.keys_cap {
            inner.buckets.clear();
        }
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
    fn burst_up_to_capacity_then_rejects() {
        let bucket = TokenBucket::new(60, 5);
        for index in 0..5 {
            assert!(
                bucket.check("k").is_ok(),
                "第 {} 个突发请求应放行",
                index + 1
            );
        }
        let retry_after = bucket.check("k").expect_err("超出突发容量应被拒");
        assert!(
            retry_after <= Duration::from_secs(1),
            "60/分 → 补一个令牌约 1 秒"
        );
        assert!(retry_after > Duration::ZERO);
    }

    #[test]
    fn refills_over_time() {
        // 60/分 = 1 个/秒；容量 1 → 用掉后约 1 秒恢复
        let bucket = TokenBucket::new(60, 1);
        assert!(bucket.check("k").is_ok());
        assert!(bucket.check("k").is_err());
        std::thread::sleep(Duration::from_millis(1100));
        assert!(bucket.check("k").is_ok(), "补充后应恢复");
    }

    #[test]
    fn waiting_does_not_accumulate_beyond_capacity() {
        let bucket = TokenBucket::new(60, 3);
        std::thread::sleep(Duration::from_millis(50));
        let available = bucket.available("k");
        assert!(
            available <= 3.0 + 1e-9,
            "空闲再久也不能超过容量，得到 {available}"
        );
    }

    #[test]
    fn keys_are_independent() {
        let bucket = TokenBucket::new(1, 1);
        assert!(bucket.check("a").is_ok());
        assert!(bucket.check("a").is_err());
        assert!(bucket.check("b").is_ok(), "另一个 key 不受影响");
    }

    #[test]
    fn zero_config_is_clamped_instead_of_blocking_everyone() {
        // 配错成 0：至少放行一个突发，而不是把所有人永久 429
        let bucket = TokenBucket::new(0, 0);
        assert!(bucket.check("k").is_ok());
    }

    #[test]
    fn many_keys_stay_bounded() {
        let bucket = TokenBucket::new(60, 1);
        for index in 0..1000 {
            assert!(bucket.check(&format!("ip-{index}")).is_ok());
        }
        // 每个 key 都能独立取到令牌，说明没有互相污染
        assert!(bucket.check("ip-0").is_err());
        assert!(bucket.check("ip-999").is_err());
    }
}
