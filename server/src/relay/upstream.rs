//! 上游转发（ADR-0047 决策 5）：只拼路径 + 转发，不解析、不改写、不缓冲响应。
//!
//! 鉴权分面：OpenAI 两个面用 `Authorization: Bearer`，Anthropic 面用 `x-api-key`
//! （官方文档与实测均如此；`anthropic-version` 被上游忽略，但 Anthropic 客户端通常
//! 会带，我们转发时补一个固定值以免上游将来收紧）。

use std::time::Duration;

use reqwest::{Client, Response};
use serde_json::Value;

use super::error::RelayError;
use super::protocol::Protocol;

/// Anthropic 兼容面的 API 版本头。上游当前忽略它，带上只为将来兼容。
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 中转用的客户端。
///
/// **刻意不设总超时**：一次生成可能持续数分钟，任何"合理默认值"都可能把正在计费的
/// 请求掐断，造成"上游扣了钱、我们收不到 usage"。只设连接超时与空闲回收。
///
/// TLS 由 native-tls 提供（Windows = schannel，Linux = OpenSSL）：这样不引入任何
/// 需要 C 编译器构建的加密 crate，见 `Cargo.toml` 里关于中文路径的说明。
pub fn build_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .expect("reqwest 客户端构建失败")
}

/// 转发一次请求，返回上游的流式响应（或错误状态，由调用方决定原样透传）。
pub async fn forward(
    client: &Client,
    base_url: &str,
    api_key: &str,
    protocol: Protocol,
    body: &Value,
) -> Result<Response, RelayError> {
    let url = format!(
        "{}{}",
        base_url.trim_end_matches('/'),
        protocol.upstream_path()
    );
    let request = client.post(&url).json(body);
    let request = match protocol {
        Protocol::Anthropic => request
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        Protocol::Responses | Protocol::ChatCompletions => request.bearer_auth(api_key),
    };

    request
        .send()
        .await
        .map_err(|e| RelayError::Upstream(format!("{url}：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_without_total_timeout() {
        // 构建本身不该 panic；真正的"无总超时"由代码结构保证（没调 .timeout()）
        let _client = build_client();
    }
}
