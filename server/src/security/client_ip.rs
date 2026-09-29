//! 客户端 IP 解析。
//!
//! 生产部署在反向代理（Caddy）后面，对端 IP 恒为 `127.0.0.1`，所以必须读转发头——
//! 但转发头是**客户端可伪造**的。因此：
//! - 只有确实位于可信代理后面时才信任它（配置 `SECURITY_TRUST_PROXY`，默认**不信任**）；
//! - 信任时取 `X-Forwarded-For` 的**最后一个**条目：那是我们自己的代理追加的，
//!   前面的条目是客户端自己写的（伪造的）。
//!
//! 不信任时一律用 TCP 对端地址——伪造头只能骗到"限流的 key"，而这个 key 拿不到任何
//! 好处（还会让攻击者把自己的真实 IP 与伪造值一起被计入）。

use std::net::SocketAddr;

use axum::http::HeaderMap;

/// 取客户端 IP（拿不到时返回 `"unknown"`，限流仍会按这个 key 生效）。
pub fn client_ip(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy: bool) -> String {
    if trust_proxy {
        if let Some(value) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
            && let Some(last) = value
                .split(',')
                .map(str::trim)
                .rev()
                .find(|entry| !entry.is_empty())
        {
            return last.to_string();
        }
        if let Some(value) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    peer.map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn peer() -> Option<SocketAddr> {
        Some("203.0.113.9:54321".parse().unwrap())
    }

    #[test]
    fn uses_peer_address_when_proxy_is_not_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        // 不信任代理时必须忽略伪造头，否则任何人换个头就能绕过限流
        assert_eq!(client_ip(&headers, peer(), false), "203.0.113.9");
    }

    #[test]
    fn takes_the_last_forwarded_entry_when_trusted() {
        let mut headers = HeaderMap::new();
        // 客户端可伪造最左边的条目；最右边是我们的代理追加的真实来源
        // （头值必须是可见 ASCII，所以这里不能用中文注释以外的字符）
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("9.9.9.9, 198.51.100.7"),
        );
        assert_eq!(client_ip(&headers, peer(), true), "198.51.100.7");
    }

    #[test]
    fn falls_back_to_real_ip_then_peer_then_unknown() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static(" 198.51.100.8 "));
        assert_eq!(client_ip(&headers, peer(), true), "198.51.100.8");

        assert_eq!(client_ip(&HeaderMap::new(), peer(), true), "203.0.113.9");
        assert_eq!(client_ip(&HeaderMap::new(), None, true), "unknown");
    }

    #[test]
    fn empty_forwarded_header_falls_through() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("  ,  "));
        assert_eq!(client_ip(&headers, peer(), true), "203.0.113.9");
    }
}
