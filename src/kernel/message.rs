//! 消息树（ADR-0007 修订）：气泡 = 一个输出 item，完成即落盘。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::kernel::contract::ToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(pub Uuid);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// 附件（图片 base64；M4 上传链路使用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub mime: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageKind {
    User {
        text: String,
        #[serde(default)]
        attachments: Vec<Attachment>,
    },
    Assistant {
        text: String,
    },
    ToolCall {
        entry: String,
        params: serde_json::Value,
        result: Result<serde_json::Value, ToolError>,
    },
    System {
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub parent_id: Option<MessageId>,
    pub kind: MessageKind,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::User {
                text: text.into(),
                attachments: Vec::new(),
            },
            created_at: chrono::Utc::now(),
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::Assistant { text: text.into() },
            created_at: chrono::Utc::now(),
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::System { text: text.into() },
            created_at: chrono::Utc::now(),
        }
    }

    pub fn tool_call(
        entry: impl Into<String>,
        params: serde_json::Value,
        result: Result<serde_json::Value, ToolError>,
    ) -> Self {
        Self {
            id: MessageId::new(),
            parent_id: None,
            kind: MessageKind::ToolCall {
                entry: entry.into(),
                params,
                result,
            },
            created_at: chrono::Utc::now(),
        }
    }
}

/// 把消息挂到链尾：parent = 最后一条消息 id。
pub fn append_to_path(messages: &mut Vec<Message>, mut msg: Message) {
    msg.parent_id = messages.last().map(|m| m.id);
    messages.push(msg);
}
