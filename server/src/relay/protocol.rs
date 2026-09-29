//! 协议适配（ADR-0047 修订 R1/R5）：下游路径 → 上游路径、用户隔离字段、请求体裁剪。
//!
//! **三面全部透传**：上游对三个协议都有同名端点（Responses / Chat Completions /
//! Anthropic 兼容面），所以这里不需要任何双向翻译——一行请求体翻译代码都没有。
//! 差异只有三处，全部集中在本模块与 [`super::sse`]：
//! 1. 上游路径映射；
//! 2. usage 的字段名与语义（见 [`super::sse`]）；
//! 3. 用户隔离字段名（R5）。

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI Responses API（本项目客户端的默认链路）
    Responses,
    /// OpenAI Chat Completions（客户端 `transport: chat_completions` 的回退链路）
    ChatCompletions,
    /// Anthropic Messages API（第三方客户端，如 Claude Code）
    Anthropic,
}

impl Protocol {
    /// 下游路径 → 协议。**同时认带 `/v1` 与不带前缀两种形态**：
    /// OpenAI SDK 与第三方客户端硬编码 `/v1/...`，而本项目客户端会把 `api_url` 尾部的
    /// `/v1` 剥掉再拼 `/responses`。
    pub fn from_path(path: &str) -> Option<Self> {
        match path {
            "/responses" | "/v1/responses" => Some(Protocol::Responses),
            "/chat/completions" | "/v1/chat/completions" => Some(Protocol::ChatCompletions),
            "/v1/messages" | "/messages" => Some(Protocol::Anthropic),
            _ => None,
        }
    }

    /// 上游路径。
    pub fn upstream_path(self) -> &'static str {
        match self {
            Protocol::Responses => "/responses",
            Protocol::ChatCompletions => "/chat/completions",
            Protocol::Anthropic => "/anthropic/v1/messages",
        }
    }

    /// 落进 `usage_events.protocol` 的标识（用于分协议看成本）。
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Responses => "responses",
            Protocol::ChatCompletions => "chat_completions",
            Protocol::Anthropic => "anthropic",
        }
    }

    /// 覆盖请求体里的模型名：**不信任客户端传来的 model**（ADR-0047 决策 5）。
    pub fn apply_model(self, body: &mut Value, model: &str) {
        // 三个协议的模型字段都叫 model，差别只在嵌套层级（Anthropic 也是顶层）
        if let Some(object) = body.as_object_mut() {
            object.insert("model".into(), Value::String(model.to_string()));
        }
    }

    /// 注入平台用户 id 到上游的隔离字段（R5）：白拿上游的 KVCache 隔离、
    /// 内容安全隔离与调度隔离。三面字段名不同，这是必须分流的一处。
    pub fn apply_user_id(self, body: &mut Value, user_id: &str) {
        let Some(object) = body.as_object_mut() else {
            return;
        };
        match self {
            Protocol::Responses => {
                object.insert("user".into(), Value::String(user_id.to_string()));
            }
            Protocol::ChatCompletions => {
                object.insert("user_id".into(), Value::String(user_id.to_string()));
            }
            Protocol::Anthropic => {
                // Anthropic 把用户标识放在 metadata.user_id
                let metadata = object
                    .entry("metadata")
                    .or_insert_with(|| Value::Object(Default::default()));
                if let Some(metadata) = metadata.as_object_mut() {
                    metadata.insert("user_id".into(), Value::String(user_id.to_string()));
                }
            }
        }
    }

    /// 客户端是否要求流式。三面都用 `stream` 字段；本网关只支持流式
    /// （usage 与中断计费都依赖事件流，非流式无法可靠旁路取用量）。
    pub fn stream_requested(self, body: &Value) -> bool {
        body.get("stream").and_then(Value::as_bool).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_both_prefixed_and_bare_paths() {
        assert_eq!(Protocol::from_path("/responses"), Some(Protocol::Responses));
        assert_eq!(
            Protocol::from_path("/v1/responses"),
            Some(Protocol::Responses)
        );
        assert_eq!(
            Protocol::from_path("/chat/completions"),
            Some(Protocol::ChatCompletions)
        );
        assert_eq!(
            Protocol::from_path("/v1/chat/completions"),
            Some(Protocol::ChatCompletions)
        );
        assert_eq!(
            Protocol::from_path("/v1/messages"),
            Some(Protocol::Anthropic)
        );
        assert_eq!(Protocol::from_path("/v1/embeddings"), None);
        assert_eq!(Protocol::from_path("/api/v1/me"), None);
    }

    #[test]
    fn upstream_paths_match_deepseek_documented_endpoints() {
        assert_eq!(Protocol::Responses.upstream_path(), "/responses");
        assert_eq!(
            Protocol::ChatCompletions.upstream_path(),
            "/chat/completions"
        );
        assert_eq!(
            Protocol::Anthropic.upstream_path(),
            "/anthropic/v1/messages",
            "Anthropic 兼容面的 base_url 是 https://api.deepseek.com/anthropic"
        );
    }

    #[test]
    fn model_is_overridden_regardless_of_client_value() {
        for protocol in [
            Protocol::Responses,
            Protocol::ChatCompletions,
            Protocol::Anthropic,
        ] {
            let mut body = json!({"model": "客户端乱填的模型", "stream": true});
            protocol.apply_model(&mut body, "deepseek-flash");
            assert_eq!(body["model"], "deepseek-flash");
        }
    }

    #[test]
    fn user_id_goes_to_the_right_field_per_protocol() {
        let user = "6f1b0a3c-0000-4000-8000-000000000001";

        let mut responses = json!({});
        Protocol::Responses.apply_user_id(&mut responses, user);
        assert_eq!(responses["user"], user);

        let mut chat = json!({});
        Protocol::ChatCompletions.apply_user_id(&mut chat, user);
        assert_eq!(chat["user_id"], user);

        let mut anthropic = json!({"metadata": {"other": 1}});
        Protocol::Anthropic.apply_user_id(&mut anthropic, user);
        assert_eq!(anthropic["metadata"]["user_id"], user);
        assert_eq!(
            anthropic["metadata"]["other"], 1,
            "注入 user_id 不该抹掉客户端已有的 metadata 字段"
        );
    }

    #[test]
    fn stream_flag_is_read_from_the_shared_field() {
        for protocol in [
            Protocol::Responses,
            Protocol::ChatCompletions,
            Protocol::Anthropic,
        ] {
            assert!(protocol.stream_requested(&json!({"stream": true})));
            assert!(!protocol.stream_requested(&json!({"stream": false})));
            assert!(!protocol.stream_requested(&json!({})));
        }
    }
}
