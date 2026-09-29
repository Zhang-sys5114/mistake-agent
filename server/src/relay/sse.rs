//! 流式旁路解析（ADR-0047 修订 R1/R3/R4）。
//!
//! 中转必须**边转发边解析**：客户端拿到的是原样字节与零额外延迟，服务端只在旁边
//! 攒出一份 usage 与终态判定用于计费。这里刻意不缓冲整条流、不改写、不重排事件。
//!
//! 容忍性来自实测（`docs/research/deepseek-api-compat.md` 附录 A）与 new-api 的工程实践：
//! - 官方会插入 `: keep-alive` 注释行与空行 → 必须跳过，且不能当成数据或终态
//! - Anthropic 面会下发 `ping`，Responses 面还有 `response.done` / `cancelled` 等变体
//!   → 未知事件必须容忍，终态按**集合**判定
//! - 三面的 usage 位置与 `input_tokens` 语义都不同 → 分别提取后归一化（R4）

use serde_json::Value;

use crate::billing::TokenUsage;

use super::protocol::Protocol;

/// 流终态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Terminal {
    /// 还没看到终态事件（流可能仍在进行，也可能被客户端掐断）
    #[default]
    Pending,
    Completed,
    Failed,
}

/// 旁路解析结果。
#[derive(Debug, Clone, Default)]
pub struct ProbeResult {
    pub usage: Option<TokenUsage>,
    /// 是否看到过任何事件——用于"上游已开始生成即计费"的判定（R3）
    pub saw_event: bool,
    /// 是否看到过输出增量（文本 / 推理 / 工具参数）
    pub saw_output: bool,
    pub terminal: Terminal,
}

/// 增量喂入的旁路解析器。只暂存"尚未收完整的一行"，不缓存整条流。
pub struct UsageProbe {
    protocol: Protocol,
    buffer: Vec<u8>,
    event: String,
    data: String,
    result: ProbeResult,
}

impl UsageProbe {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            buffer: Vec::new(),
            event: String::new(),
            data: String::new(),
            result: ProbeResult::default(),
        }
    }

    /// 喂入一段上游字节（可以任意切分，包括切在行中间）。
    pub fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some(position) = self.buffer.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1])
                .trim_end_matches('\r')
                .to_string();
            self.line(&line);
        }
    }

    /// 收尾：把残留的半行丢弃（截断的流不该被当成数据），返回结果。
    pub fn finish(mut self) -> ProbeResult {
        self.buffer.clear();
        self.result
    }

    fn line(&mut self, line: &str) {
        if line.is_empty() {
            self.dispatch();
            return;
        }
        // 注释行（`: keep-alive`）：官方保活机制，跳过
        if line.starts_with(':') {
            return;
        }
        if let Some(value) = line.strip_prefix("event:") {
            self.event = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            if !self.data.is_empty() {
                self.data.push('\n');
            }
            self.data.push_str(value.trim_start());
        }
        // 其余字段（id: / retry: 等）忽略，保证未知形状不会打断解析
    }

    fn dispatch(&mut self) {
        if self.event.is_empty() && self.data.is_empty() {
            return;
        }
        let event = std::mem::take(&mut self.event);
        let data = std::mem::take(&mut self.data);
        self.result.saw_event = true;

        match self.protocol {
            Protocol::Responses => self.observe_responses(&data),
            Protocol::ChatCompletions => self.observe_chat(&data),
            Protocol::Anthropic => self.observe_anthropic(&event, &data),
        }
    }

    fn observe_responses(&mut self, data: &str) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        // usage 落在 response.usage（顶层没有 usage，但兼容端可能给顶层，两个都认）
        if let Some(usage) = value
            .pointer("/response/usage")
            .filter(|usage| usage.is_object())
            .or_else(|| value.get("usage").filter(|usage| usage.is_object()))
        {
            self.result.usage = Some(responses_usage(usage));
        }
        match value.get("type").and_then(Value::as_str) {
            // incomplete 也携带完整 usage（研究报告建议纳入），仍算"未失败"
            Some("response.completed" | "response.done" | "response.incomplete") => {
                self.result.terminal = Terminal::Completed;
            }
            Some("response.failed" | "response.cancelled" | "response.canceled") => {
                self.result.terminal = Terminal::Failed;
            }
            // 所有增量事件都以 .delta 结尾，都是上游计费的输出
            Some(kind) if kind.ends_with(".delta") => self.result.saw_output = true,
            _ => {}
        }
    }

    fn observe_chat(&mut self, data: &str) {
        if data.trim() == "[DONE]" {
            self.result.terminal = Terminal::Completed;
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
            self.result.usage = Some(chat_usage(usage));
        }
        if value.get("error").is_some() {
            self.result.terminal = Terminal::Failed;
        }
        if let Some(choices) = value.get("choices").and_then(Value::as_array) {
            for choice in choices {
                let delta = &choice["delta"];
                let text = |key: &str| {
                    delta
                        .get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.is_empty())
                };
                if text("content") || text("reasoning_content") || delta.get("tool_calls").is_some()
                {
                    self.result.saw_output = true;
                }
            }
        }
    }

    fn observe_anthropic(&mut self, event: &str, data: &str) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        match event {
            // 输入在这里，此时 output_tokens 恒为 0
            "message_start" => {
                if let Some(usage) = value
                    .pointer("/message/usage")
                    .filter(|usage| usage.is_object())
                {
                    self.result.usage = Some(anthropic_usage(usage));
                }
            }
            // 终值在这里，覆盖 message_start 那份
            "message_delta" => {
                if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
                    self.result.usage = Some(anthropic_usage(usage));
                }
            }
            "message_stop" => self.result.terminal = Terminal::Completed,
            "error" => self.result.terminal = Terminal::Failed,
            "content_block_delta" => self.result.saw_output = true,
            // 实测会下发 ping；未知事件一律容忍
            _ => {}
        }
    }
}

fn u64_field(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Responses：`input_tokens` **已包含**缓存命中。
fn responses_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        input_total: u64_field(usage, "input_tokens"),
        cached: u64_field(&usage["input_tokens_details"], "cached_tokens"),
        output: u64_field(usage, "output_tokens"),
        reasoning: u64_field(&usage["output_tokens_details"], "reasoning_tokens"),
    }
}

/// Chat Completions：`prompt_tokens` 已含缓存命中，另有 hit/miss 拆分。
fn chat_usage(usage: &Value) -> TokenUsage {
    let cached = if usage.get("prompt_tokens_details").is_some() {
        u64_field(&usage["prompt_tokens_details"], "cached_tokens")
    } else {
        u64_field(usage, "prompt_cache_hit_tokens")
    };
    TokenUsage {
        input_total: u64_field(usage, "prompt_tokens"),
        cached,
        output: u64_field(usage, "completion_tokens"),
        reasoning: u64_field(&usage["completion_tokens_details"], "reasoning_tokens"),
    }
}

/// Anthropic：`input_tokens` **不含**缓存读取，总输入 = 三者相加（R4 的语义差）。
fn anthropic_usage(usage: &Value) -> TokenUsage {
    let cache_read = u64_field(usage, "cache_read_input_tokens");
    let cache_creation = u64_field(usage, "cache_creation_input_tokens");
    TokenUsage {
        input_total: u64_field(usage, "input_tokens")
            .saturating_add(cache_read)
            .saturating_add(cache_creation),
        cached: cache_read,
        output: u64_field(usage, "output_tokens"),
        // Anthropic 形状没有 reasoning 字段：thinking 以 content block 形式计在 output 里
        reasoning: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 三个夹具都是 2026-09-29 对真实 DeepSeek 端点的抓包（原样字节，无密钥残留）。
    const RESPONSES_FIXTURE: &str =
        include_str!("../../tests/fixtures/deepseek_responses_20260929.sse");
    const CHAT_FIXTURE: &str =
        include_str!("../../tests/fixtures/deepseek_chat_completions_20260929.sse");
    const ANTHROPIC_FIXTURE: &str =
        include_str!("../../tests/fixtures/deepseek_anthropic_20260929.sse");

    fn probe(protocol: Protocol, raw: &str) -> ProbeResult {
        let mut probe = UsageProbe::new(protocol);
        probe.push(raw.as_bytes());
        probe.finish()
    }

    #[test]
    fn responses_fixture_yields_documented_usage() {
        let result = probe(Protocol::Responses, RESPONSES_FIXTURE);
        let usage = result.usage.expect("夹具里应有 usage");
        assert_eq!(usage.input_total, 12);
        assert_eq!(usage.cached, 0);
        assert_eq!(usage.output, 1);
        assert_eq!(usage.reasoning, 0);
        assert_eq!(result.terminal, Terminal::Completed);
        assert!(result.saw_event && result.saw_output);
    }

    #[test]
    fn chat_fixture_yields_documented_usage() {
        let result = probe(Protocol::ChatCompletions, CHAT_FIXTURE);
        let usage = result.usage.expect("夹具里应有 usage");
        assert_eq!(usage.input_total, 38, "prompt_tokens");
        assert_eq!(usage.cached, 0);
        assert_eq!(usage.output, 36, "completion_tokens");
        assert_eq!(
            usage.reasoning, 34,
            "completion_tokens_details.reasoning_tokens"
        );
        assert_eq!(result.terminal, Terminal::Completed, "[DONE] 是终态");
    }

    #[test]
    fn anthropic_fixture_takes_the_message_delta_final_value() {
        let result = probe(Protocol::Anthropic, ANTHROPIC_FIXTURE);
        let usage = result.usage.expect("夹具里应有 usage");
        assert_eq!(usage.input_total, 38);
        assert_eq!(usage.cached, 0);
        // message_start 里 output_tokens=0，必须被 message_delta 的终值覆盖
        assert_eq!(usage.output, 30);
        assert_eq!(result.terminal, Terminal::Completed);
    }

    #[test]
    fn chunk_boundaries_do_not_matter() {
        // 逐字节喂入：解析器必须只依赖行完整性，不能依赖 chunk 切法
        let mut probe = UsageProbe::new(Protocol::Responses);
        for byte in RESPONSES_FIXTURE.as_bytes() {
            probe.push(&[*byte]);
        }
        let result = probe.finish();
        assert_eq!(result.usage.unwrap().input_total, 12);
        assert_eq!(result.terminal, Terminal::Completed);
    }

    #[test]
    fn crlf_line_endings_are_accepted() {
        // 夹具在工作区可能是 CRLF（git autocrlf），解析器不能因此失效
        let raw = RESPONSES_FIXTURE.replace('\n', "\r\n");
        let result = probe(Protocol::Responses, &raw);
        assert_eq!(result.usage.unwrap().input_total, 12);
        assert_eq!(result.terminal, Terminal::Completed);
    }

    #[test]
    fn keep_alive_comments_and_unknown_events_are_tolerated() {
        let raw = concat!(
            ": keep-alive\n",
            "\n",
            "event: ping\n",
            "data: {\"type\":\"ping\"}\n",
            "\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"2\"}\n",
            "\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",",
            "\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}}\n",
            "\n",
        );
        let result = probe(Protocol::Responses, raw);
        assert_eq!(result.terminal, Terminal::Completed);
        assert_eq!(result.usage.unwrap().input_total, 7);
        assert!(result.saw_output, "文本增量应被记为输出");
    }

    #[test]
    fn responses_failed_terminal_is_detected() {
        let raw = concat!(
            "event: response.failed\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\",",
            "\"usage\":{\"input_tokens\":11,\"output_tokens\":0}}}\n\n",
        );
        let result = probe(Protocol::Responses, raw);
        assert_eq!(result.terminal, Terminal::Failed);
        assert_eq!(result.usage.unwrap().input_total, 11, "失败也带 usage");
    }

    #[test]
    fn anthropic_input_tokens_are_normalized_to_include_cache() {
        // Anthropic 的 input_tokens 不含缓存读取，归一化后必须相加（R4）
        let raw = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":",
            "{\"input_tokens\":100,\"cache_read_input_tokens\":900,",
            "\"cache_creation_input_tokens\":50,\"output_tokens\":0}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":",
            "{\"input_tokens\":100,\"cache_read_input_tokens\":900,",
            "\"cache_creation_input_tokens\":50,\"output_tokens\":20}}\n\n",
        );
        let result = probe(Protocol::Anthropic, raw);
        let usage = result.usage.unwrap();
        assert_eq!(usage.input_total, 1050, "100 + 900 + 50");
        assert_eq!(usage.cached, 900);
        assert_eq!(usage.output, 20);
    }

    #[test]
    fn truncated_stream_still_reports_what_was_seen() {
        // 客户端掐断：没有终态、没有 usage，但已看到输出——计费侧据此收最低 1 次（R3）
        let raw = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"答\"}\n\n",
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.de",
        );
        let result = probe(Protocol::Responses, raw);
        assert_eq!(result.terminal, Terminal::Pending);
        assert!(result.usage.is_none());
        assert!(result.saw_event && result.saw_output);
    }
}
