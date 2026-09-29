//! 安全护栏：请求限流 + 失败封禁 + 全局并发上限。
//!
//! 三层各管一件事，互不替代：
//!
//! 1. **令牌桶限流**（[`ratelimit`]）管"持续速率"——账号面按 IP、中转面按用户与按 IP。
//!    选桶不选滑动窗口：突发友好（学生一轮工具调用会连发数个请求）且状态 O(1)/key。
//! 2. **失败封禁**（[`lockout`]）管"反复失败"——登录按 IP 与按账号累计失败后临时封禁，
//!    即 fail2ban 语义的进程内版；同时输出固定格式日志行，供外部 fail2ban 做更长期封禁
//!    （配置见 `server/deploy/fail2ban/`）。
//! 3. **全局并发上限**管"同时在飞数"——令牌桶限制的是速率，限不住"同时挂着一堆长流式
//!    请求"，所以需要独立的计数器来保护上游账号与进程容量。
//!
//! 全部是进程内状态：与单实例部署匹配。**水平扩展时必须挪到共享存储**
//! （ADR-0047 修订 R9 已预先记录这条）。

mod client_ip;
mod lockout;
mod ratelimit;

pub use client_ip::client_ip;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use lockout::FailureGuard;
use ratelimit::TokenBucket;

/// 安全相关配置（ADR-0047 修订 R9 的"限流参数取值"落地）。
#[derive(Debug, Clone)]
pub struct SecuritySettings {
    /// 是否信任 `X-Forwarded-For` / `X-Real-IP`。**默认不信任**：只有在反向代理后面
    /// 才该打开，否则任何人都能伪造来源 IP 绕过按 IP 的限流。
    pub trust_proxy: bool,
    /// 账号面每 IP 每分钟请求数
    pub auth_rate_per_minute: u32,
    /// 账号面的突发容量
    pub auth_burst: u32,
    /// 中转面每用户 / 每 IP 每分钟请求数
    pub relay_rate_per_minute: u32,
    /// 中转面的突发容量（学生一轮工具调用会连发几个请求，容量要能吸收）
    pub relay_burst: u32,
    /// 登录失败阈值（窗口内达到即封禁）
    pub login_max_failures: u32,
    /// 失败计数窗口
    pub login_failure_window_secs: u64,
    /// 封禁时长
    pub login_block_secs: u64,
    /// 全局同时在飞的中转请求上限
    pub max_concurrent_global: i32,
}

/// 默认值：都能被环境变量覆盖，取向是"保护上游与账号，但不误伤正常学生"。
pub const DEFAULT_TRUST_PROXY: bool = false;
pub const DEFAULT_AUTH_RATE_PER_MINUTE: u32 = 30;
pub const DEFAULT_AUTH_BURST: u32 = 10;
pub const DEFAULT_RELAY_RATE_PER_MINUTE: u32 = 60;
/// 容量 = 1 分钟的持续速率：允许"一分钟内用满额度"的突发，正是学生连续提问的形态。
pub const DEFAULT_RELAY_BURST: u32 = 60;
pub const DEFAULT_LOGIN_MAX_FAILURES: u32 = 5;
pub const DEFAULT_LOGIN_FAILURE_WINDOW_SECS: u64 = 600;
pub const DEFAULT_LOGIN_BLOCK_SECS: u64 = 900;
pub const DEFAULT_MAX_CONCURRENT_GLOBAL: i32 = 200;

/// 运行期安全状态（由 `AppState` 持有）。
pub struct SecurityState {
    settings: SecuritySettings,
    auth_by_ip: TokenBucket,
    relay_by_user: TokenBucket,
    relay_by_ip: TokenBucket,
    login_failures: FailureGuard,
    global_in_flight: Mutex<i32>,
}

impl SecurityState {
    pub fn new(settings: SecuritySettings) -> Self {
        let auth_by_ip = TokenBucket::new(settings.auth_rate_per_minute, settings.auth_burst);
        let relay_by_user = TokenBucket::new(settings.relay_rate_per_minute, settings.relay_burst);
        let relay_by_ip = TokenBucket::new(settings.relay_rate_per_minute, settings.relay_burst);
        let login_failures = FailureGuard::new(
            settings.login_max_failures,
            Duration::from_secs(settings.login_failure_window_secs),
            Duration::from_secs(settings.login_block_secs),
        );
        Self {
            settings,
            auth_by_ip,
            relay_by_user,
            relay_by_ip,
            login_failures,
            global_in_flight: Mutex::new(0),
        }
    }

    pub fn settings(&self) -> &SecuritySettings {
        &self.settings
    }

    /// 账号面（注册/登录/账号查询）按 IP 限流。
    pub fn check_auth_rate(&self, ip: &str) -> Result<(), Duration> {
        self.auth_by_ip.check(ip)
    }

    /// 中转面：用户与 IP 两道桶都要过（同一个 NAT 后的多个学生各自有账号，
    /// 但共享出口 IP——所以按 IP 的桶容量与速率都比按用户宽松是合理的，
    /// 这里用同一组参数，真正偏紧的是按用户的桶）。
    pub fn check_relay_rate(&self, user_id: Uuid, ip: &str) -> Result<(), Duration> {
        if let Err(retry_after) = self.relay_by_user.check(&user_id.to_string()) {
            // 被限流是用户会来问的事：把"桶里还剩多少"记进日志便于定位
            tracing::debug!(
                user_id = %user_id,
                remaining_tokens = self.relay_by_user.available(&user_id.to_string()),
                "中转限流：用户桶已空"
            );
            return Err(retry_after);
        }
        // 通过用户桶后再查 IP 桶：用户桶不过就不该消耗 IP 桶的令牌
        self.relay_by_ip.check(ip)
    }

    /// 登录是否被允许（IP 与账号任一被封都拒绝）。
    pub fn login_allowed(&self, ip: &str, email: &str) -> Result<(), Duration> {
        let by_ip = self.login_failures.check(&ip_key(ip));
        let by_email = self.login_failures.check(&email_key(email));
        match (by_ip, by_email) {
            (Err(after), _) => Err(after),
            (_, Err(after)) => Err(after),
            _ => Ok(()),
        }
    }

    /// 记一次登录失败；返回是否**刚触发封禁**（用于日志与告警）。
    pub fn record_login_failure(&self, ip: &str, email: &str) -> bool {
        let blocked_by_ip = self.login_failures.record_failure(&ip_key(ip));
        let blocked_by_email = self.login_failures.record_failure(&email_key(email));
        blocked_by_ip || blocked_by_email
    }

    /// 登录成功即清零两侧计数（避免正常用户被自己偶尔的输错拖进封禁）。
    pub fn record_login_success(&self, ip: &str, email: &str) {
        self.login_failures.record_success(&ip_key(ip));
        self.login_failures.record_success(&email_key(email));
    }

    /// 占用一个全局并发名额；已满返回 `None`（调用方映射为 429）。
    pub fn acquire_global_slot(self: &Arc<Self>) -> Option<GlobalSlot> {
        let limit = self.settings.max_concurrent_global.max(1);
        let mut in_flight = self.lock_global();
        if *in_flight >= limit {
            return None;
        }
        *in_flight += 1;
        Some(GlobalSlot {
            state: Arc::clone(self),
        })
    }

    /// 当前全局在飞数（诊断用）。
    pub fn global_in_flight(&self) -> i32 {
        *self.lock_global()
    }

    fn lock_global(&self) -> std::sync::MutexGuard<'_, i32> {
        self.global_in_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 全局名额凭据：离开作用域即归还（含任务被取消的情况）。
pub struct GlobalSlot {
    state: Arc<SecurityState>,
}

impl Drop for GlobalSlot {
    fn drop(&mut self) {
        let mut in_flight = self.state.lock_global();
        *in_flight -= 1;
        if *in_flight < 0 {
            // 不该发生；夹到 0 保证不会因为一次记账错误永久锁死全部中转
            *in_flight = 0;
        }
    }
}

fn ip_key(ip: &str) -> String {
    format!("ip:{ip}")
}

fn email_key(email: &str) -> String {
    format!("email:{}", email.trim().to_lowercase())
}

/// 客户端来源 IP（提取器）。
///
/// 直接读 `parts.extensions` 里的 `ConnectInfo` 而不是把它做成必填提取器：
/// 集成测试构造的请求没有连接信息，做成必填会让"没接 TCP 对端"变成 500；
/// 做成可选则只是退化成 `unknown`（限流仍按这个 key 生效）。
pub struct ClientIp(pub String);

impl axum::extract::FromRequestParts<crate::http::AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &crate::http::AppState,
    ) -> Result<Self, Self::Rejection> {
        use axum::extract::ConnectInfo;
        let peer = parts
            .extensions
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|info| info.0);
        let trust_proxy = state.security.settings().trust_proxy;
        Ok(ClientIp(client_ip(&parts.headers, peer, trust_proxy)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> SecuritySettings {
        SecuritySettings {
            trust_proxy: false,
            auth_rate_per_minute: 30,
            auth_burst: 2,
            relay_rate_per_minute: 60,
            relay_burst: 3,
            login_max_failures: 3,
            login_failure_window_secs: 600,
            login_block_secs: 900,
            max_concurrent_global: 2,
        }
    }

    #[test]
    fn relay_rate_checks_both_user_and_ip_buckets() {
        let state = SecurityState::new(settings());
        let user = Uuid::new_v4();
        // 突发容量 3：前三次过
        for _ in 0..3 {
            assert!(state.check_relay_rate(user, "1.1.1.1").is_ok());
        }
        assert!(state.check_relay_rate(user, "1.1.1.1").is_err());

        // 另一个用户但从同一 IP 发起：IP 桶已耗尽，应被拒
        let other = Uuid::new_v4();
        assert!(
            state.check_relay_rate(other, "1.1.1.1").is_err(),
            "共享出口 IP 的持续滥用应被 IP 桶拦住"
        );
        // 换个 IP 的新用户不受影响
        assert!(state.check_relay_rate(other, "2.2.2.2").is_ok());
    }

    #[test]
    fn login_failures_block_both_ip_and_account() {
        let state = SecurityState::new(settings());
        let email = "Student@Example.test";
        for _ in 0..2 {
            assert!(!state.record_login_failure("1.1.1.1", email));
        }
        assert!(
            state.record_login_failure("1.1.1.1", email),
            "第 3 次应触发封禁"
        );
        let retry_after = state.login_allowed("1.1.1.1", email).expect_err("应被封禁");
        assert!(retry_after <= Duration::from_secs(900));

        // 同一账号从别的 IP 也被封（大小写不同也是同一个账号）
        assert!(
            state
                .login_allowed("9.9.9.9", "student@example.test")
                .is_err()
        );
        // 无关账号从无关 IP 不受影响
        assert!(state.login_allowed("9.9.9.9", "other@example.test").is_ok());
    }

    #[test]
    fn successful_login_clears_failures() {
        let state = SecurityState::new(settings());
        state.record_login_failure("1.1.1.1", "a@b.test");
        state.record_login_failure("1.1.1.1", "a@b.test");
        state.record_login_success("1.1.1.1", "a@b.test");
        assert!(state.login_allowed("1.1.1.1", "a@b.test").is_ok());
    }

    #[test]
    fn global_slots_are_capped_and_returned_on_drop() {
        let state = Arc::new(SecurityState::new(settings()));
        let first = state.acquire_global_slot().expect("第 1 个名额");
        let _second = state.acquire_global_slot().expect("第 2 个名额");
        assert!(state.acquire_global_slot().is_none(), "上限为 2");
        assert_eq!(state.global_in_flight(), 2);

        drop(first);
        assert_eq!(state.global_in_flight(), 1);
        assert!(state.acquire_global_slot().is_some(), "归还后应能再占用");
    }

    #[test]
    fn auth_rate_is_per_ip() {
        let state = SecurityState::new(settings());
        assert!(state.check_auth_rate("1.1.1.1").is_ok());
        assert!(state.check_auth_rate("1.1.1.1").is_ok());
        assert!(state.check_auth_rate("1.1.1.1").is_err(), "突发容量 2");
        assert!(state.check_auth_rate("2.2.2.2").is_ok());
    }
}
