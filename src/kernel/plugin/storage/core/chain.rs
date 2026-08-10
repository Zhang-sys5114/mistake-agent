//! 消息树链辅助（active_session/last_message_id/active_chain）。

use super::*;

pub fn active_session(metas: &[SessionMeta]) -> Option<SessionMeta> {
    metas
        .iter()
        .find(|m| m.status == SessionStatus::Active)
        .cloned()
}

pub fn last_message_id(messages: &[Message]) -> Option<MessageId> {
    messages.last().map(|m| m.id)
}

/// 沿 parent 链回溯构造活跃路径（根 → 末端）。
/// `active_path` 为 None（旧数据/线性会话）时退化为完整消息列表。
pub fn active_chain(messages: &[Message], active_path: Option<MessageId>) -> Vec<Message> {
    let Some(end) = active_path else {
        return messages.to_vec();
    };
    let by_id: HashMap<MessageId, Message> = messages.iter().map(|m| (m.id, m.clone())).collect();
    if !by_id.contains_key(&end) {
        return messages.to_vec();
    }
    let mut chain = Vec::new();
    let mut cur = Some(end);
    while let Some(id) = cur {
        match by_id.get(&id) {
            Some(m) => {
                cur = m.parent_id;
                chain.push(m.clone());
            }
            None => break,
        }
    }
    chain.reverse();
    chain
}
