//! RPC 协议类型（ADR-0013 / Q15）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::kernel::agent::session::SessionKey;
use crate::kernel::events::Event;
use crate::kernel::message::MessageId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Method {
    SendUserMessage {
        text: String,
        /// 显式工具调用：强制 LLM 首轮调用指定工具（不绕过 LLM）。
        #[serde(default)]
        force_tool: Option<ForcedToolRequest>,
        /// 暂存文件路径列表（mistake-agent- 前缀临时路径）：模型读图/判分时作为 file 参数。
        #[serde(default)]
        file: Vec<String>,
        /// 持久附件列表（数据根目录 uploads/ 副本）：落进消息文本供前端展示。
        #[serde(default)]
        asset: Vec<AttachmentInfo>,
    },
    TriggerCommand {
        entry: String,
        params: Value,
    },
    EditMessage {
        message_id: MessageId,
        text: String,
    },
    SwitchBranch {
        message_id: MessageId,
    },
    Abort,
    GetState,
    GetSettings,
    SetSettings {
        patch: crate::kernel::settings::SettingsPatch,
    },
    /// GUI 验算执行端回执（compute 桥接）。
    ComputeResult {
        /// 与 RPC 请求帧顶层 id 区分，前端按 compute_id 回执。
        #[serde(rename = "compute_id")]
        id: u64,
        stdout: String,
        stderr: String,
        duration_ms: u64,
    },
    /// 会话列表（GUI 会话历史页）。
    ListSessions,
    /// 读取指定会话完整消息树（GUI 历史浏览/分支回放）。
    ReadSession {
        key: SessionKey,
    },
    /// 用户可调工具/命令清单（GUI 工具面板）。
    ListTools,
    /// 连通性自检：主/视觉模型轻量调用（OOBE 引导"测试连接"用）。
    /// 可携带表单中的临时 api_key（不落盘，仅本次请求生效）；model 取 "main"/"vision"。
    TestConnection {
        #[serde(default)]
        api_key: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    /// 账户余额查询：DeepSeek /user/balance + SiliconFlow /user/info（只读，不落盘 key）。
    CheckBalance,
    /// 聊天上下文缓存命中统计（主模型回合调用累计，按会话 + 全局）。
    GetCacheStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub id: u64,
    #[serde(flatten)]
    pub method: Method,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl RpcError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcFrame {
    Response {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<RpcError>,
    },
    Event {
        event: Event,
    },
}

/// 显式工具调用请求：entry 为内部全名（namespace::tool），hint 为用户输入的可选参数文本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForcedToolRequest {
    pub entry: String,
    #[serde(default)]
    pub hint: Option<String>,
    /// 前端原始展示文本（如「翻看记忆：数学/向量组…」）；缺省时 kernel 按 title＋hint 兜底，
    /// 落盘到 user 消息的 display_text，重开会话后渲染仍友好。
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub path: String,
    pub name: String,
}
