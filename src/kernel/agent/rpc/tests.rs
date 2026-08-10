use super::*;
use crate::kernel::agent::session::SessionMeta;
use crate::kernel::plugin::storage::MemoryStorage;

#[tokio::test]
async fn switch_tool_call_not_persisted_and_children_reparented() {
    let store: Arc<dyn SessionStore> = Arc::new(MemoryStorage::new());
    let key = SessionKey::new();
    store
        .create_session(&key, &SessionMeta::new(key))
        .await
        .unwrap();
    let user = Message::user("帮我批改数学作业");
    store.append_message(&key, &user).await.unwrap();

    let mut switch = Message::tool_call(
        "session::switch",
        json!({"goal": "批改英语作业"}),
        Ok(json!({"switched": true})),
    );
    switch.parent_id = Some(user.id);
    let mut answer = Message::assistant("好的，先切换到英语作业");
    answer.parent_id = Some(switch.id);
    let answer_id = answer.id;

    let last = persist_turn_messages(&store, &key, &[switch, answer], None)
        .await
        .unwrap();
    assert_eq!(last, Some(answer_id));

    let path = store.read_path(&key).await.unwrap();
    assert_eq!(path.len(), 2, "切换控制消息不应落盘");
    assert!(!path[1].is_switch_tool_call());
    assert_eq!(
        path[1].parent_id,
        Some(user.id),
        "子消息父链应重接到切换前最后一条"
    );
}
