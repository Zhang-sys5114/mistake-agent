//! memory 插件：跨会话记忆路由（save/show/remove，路径层级组织）。
//!
//! 与内核 memory 服务的分工：路径校验、文件持久化、事件与审计由内核
//! MemoryService 负责；本插件只做参数解析与错误映射（ADR-0008）。

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::dispatch::ToolCallContext;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
use crate::kernel::services::{MemoryError, MemoryHandle, MemoryPath, MemoryView, ServiceId};

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SaveParams {
    /// 记忆路径（层级：学科/知识点/条目），如"数学/函数/二次函数 顶点公式"；缺省 = 会话/自动保存。
    pub filename: Option<String>,
    /// 要记住的内容（一句话结论或要点）；缺省 = 由模型总结当前会话上下文后填入。
    pub content: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ShowParams {
    /// 可选：省略 = 列出全部记忆条目名；带 = 查看具体条目内容。
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RemoveParams {
    /// 要删除的记忆条目或整棵子树路径。
    pub filename: String,
}

pub struct MemoryPlugin;

impl UserPlugin for MemoryPlugin {
    fn info() -> Info {
        Info {
            namespace: "memory".into(),
            requires: vec![ServiceId::Memory],
            tools: vec![
                ToolDef {
                    name: "save".into(),
                    user_visible: true,
                    title: Some("记住一条笔记".into()),
                    group: Some("记忆".into()),
                    description: "把重要结论/学习要点保存到跨会话记忆。用法：memory::save <想保存的内容>（可选参数；内容缺省时请总结当前会话要点填入 content）".into(),
                    params: schemars::schema_for!(SaveParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:content-save".into()),
                },
                ToolDef {
                    name: "show".into(),
                    user_visible: true,
                    title: Some("翻看记忆".into()),
                    group: Some("记忆".into()),
                    description: "查看记忆：用法 memory::show <记忆片段>（可选参数；省略 = 列出全部条目名）".into(),
                    params: schemars::schema_for!(ShowParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:book-open-variant".into()),
                },
                ToolDef {
                    name: "remove".into(),
                    user_visible: true,
                    title: Some("删除记忆".into()),
                    group: Some("记忆".into()),
                    description: "删除记忆条目或整棵子树（仅用户可调，需确认）。用法：memory::remove <记忆片段>".into(),
                    params: schemars::schema_for!(RemoveParams),
                    policy: CallerPolicy::UserOnly,
                    timeout: None,
                    icon: Some("mdi:delete".into()),
                },
            ],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let memory = ctx
            .handles
            .memory()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Memory 句柄".into()))?;

        let mem_save = memory.clone();
        ctx.registrar.tool(
            "save",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_save.clone();
                Box::pin(async move { save_handler(memory, params).await })
            }),
        )?;

        let mem_show = memory.clone();
        ctx.registrar.tool(
            "show",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_show.clone();
                Box::pin(async move { show_handler(memory, params).await })
            }),
        )?;

        let mem_remove = memory.clone();
        ctx.registrar.tool(
            "remove",
            Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let memory = mem_remove.clone();
                Box::pin(async move { remove_handler(memory, params).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<MemoryPlugin>()
}

async fn save_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: SaveParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let filename = p.filename.unwrap_or_else(|| "会话/自动保存".into());
    let content = p.content.unwrap_or_default();
    if content.trim().is_empty() {
        return Err(ToolError::handler(
            "未提供要保存的内容。用法：memory::save <想保存的内容>；也可以由模型总结当前会话要点后填入 content。",
        ));
    }
    let path = MemoryPath::parse(&filename).map_err(map_memory_error)?;
    memory
        .save(&path, &content)
        .await
        .map_err(map_memory_error)?;
    Ok(json!({ "ok": true, "path": path.as_str() }))
}

async fn show_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: ShowParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    let path = p
        .filename
        .as_deref()
        .map(MemoryPath::parse)
        .transpose()
        .map_err(map_memory_error)?;
    match memory.show(path.as_ref()).await.map_err(map_memory_error)? {
        MemoryView::Listing(entries) => Ok(json!({
            "count": entries.len(),
            "entries": entries,
        })),
        MemoryView::Entry { path, content } => Ok(json!({
            "path": path.as_str(),
            "content": content,
        })),
    }
}

async fn remove_handler(memory: MemoryHandle, params: Value) -> Result<Value, ToolError> {
    let p: RemoveParams =
        serde_json::from_value(params).map_err(|e| ToolError::invalid_params(e.to_string()))?;
    if p.filename.trim().is_empty() {
        return Err(ToolError::invalid_params(
            "remove 必须提供 filename 以确认删除目标",
        ));
    }
    let path = MemoryPath::parse(&p.filename).map_err(map_memory_error)?;
    memory.remove(&path).await.map_err(map_memory_error)?;
    Ok(json!({ "ok": true, "removed": path.as_str() }))
}

fn map_memory_error(e: MemoryError) -> ToolError {
    match e {
        MemoryError::InvalidPath(m) => ToolError::invalid_params(m),
        MemoryError::NotFound(m) => ToolError::handler(format!("记忆条目不存在：{m}")),
        MemoryError::Io(m) => ToolError::handler(format!("记忆读写失败：{m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeMemory {
        entries: Mutex<BTreeMap<String, String>>,
    }

    #[async_trait::async_trait]
    impl crate::kernel::services::MemoryService for FakeMemory {
        async fn save(&self, path: &MemoryPath, content: &str) -> Result<(), MemoryError> {
            self.entries
                .lock()
                .expect("poisoned")
                .insert(path.as_str(), content.to_string());
            Ok(())
        }
        async fn show(&self, path: Option<&MemoryPath>) -> Result<MemoryView, MemoryError> {
            let entries = self.entries.lock().expect("poisoned");
            match path {
                None => Ok(MemoryView::Listing(entries.keys().cloned().collect())),
                Some(p) => entries
                    .get(&p.as_str())
                    .cloned()
                    .map(|content| MemoryView::Entry {
                        path: p.clone(),
                        content,
                    })
                    .ok_or_else(|| MemoryError::NotFound(p.as_str())),
            }
        }
        async fn remove(&self, path: &MemoryPath) -> Result<(), MemoryError> {
            let prefix = format!("{}/", path.as_str());
            let mut entries = self.entries.lock().expect("poisoned");
            let keys: Vec<String> = entries
                .keys()
                .filter(|k| *k == &path.as_str() || k.starts_with(&prefix))
                .cloned()
                .collect();
            if keys.is_empty() {
                return Err(MemoryError::NotFound(path.as_str()));
            }
            for k in keys {
                entries.remove(&k);
            }
            Ok(())
        }
    }

    fn handle() -> MemoryHandle {
        MemoryHandle::new(std::sync::Arc::new(FakeMemory::default()))
    }

    #[tokio::test]
    async fn save_show_roundtrip() {
        let mem = handle();
        let out = save_handler(
            mem.clone(),
            json!({"filename": "数学/函数/二次函数 顶点公式", "content": "顶点公式 x=-b/2a"}),
        )
        .await
        .unwrap();
        assert_eq!(out["ok"], true);

        let listing = show_handler(mem.clone(), json!({})).await.unwrap();
        assert_eq!(listing["count"], 1);
        assert_eq!(listing["entries"][0], "数学/函数/二次函数 顶点公式");

        let detail = show_handler(
            mem.clone(),
            json!({"filename": "数学/函数/二次函数 顶点公式"}),
        )
        .await
        .unwrap();
        assert!(detail["content"].as_str().unwrap().contains("顶点公式"));
    }

    #[tokio::test]
    async fn remove_requires_and_deletes_subtree() {
        let mem = handle();
        save_handler(
            mem.clone(),
            json!({"filename": "数学/几何/勾股", "content": "a²+b²=c²"}),
        )
        .await
        .unwrap();
        save_handler(
            mem.clone(),
            json!({"filename": "数学/几何/相似", "content": "对应边成比例"}),
        )
        .await
        .unwrap();

        // remove 参数必填：缺 filename 应报 invalid_params。
        let err = remove_handler(mem.clone(), json!({})).await.unwrap_err();
        assert_eq!(
            err.code,
            crate::kernel::contract::ToolErrorCode::InvalidParams
        );

        let out = remove_handler(mem.clone(), json!({"filename": "数学/几何"}))
            .await
            .unwrap();
        assert_eq!(out["ok"], true);
        let listing = show_handler(mem, json!({})).await.unwrap();
        assert_eq!(listing["count"], 0);
    }

    #[tokio::test]
    async fn path_escape_rejected() {
        let mem = handle();
        let err = save_handler(mem, json!({"filename": "数学/../秘密", "content": "x"}))
            .await
            .unwrap_err();
        assert_eq!(
            err.code,
            crate::kernel::contract::ToolErrorCode::InvalidParams
        );
    }
}
