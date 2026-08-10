mod inmem;
mod store;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::KernelContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::{
    Domain, DomainIo, MemoryError, MemoryHandle, MemoryPath, MemoryService, MemoryView, RelPath,
    ServiceId,
};
use crate::kernel::registry::{KernelDescriptor, KernelPlugin};

pub use inmem::InMemoryMemory;
pub use store::FileMemoryService;
#[cfg(test)]
pub(crate) use store::rel_to_memory_name;

// ---------- 内核插件入口（ADR-0035）：memory::save/show/remove ----------
//
// 路径校验、文件持久化、事件与审计由本模块 MemoryService / MemoryHandle 负责；
// 入口层只做参数解析与错误映射（ADR-0008）。工具经 KernelPlugin 契约注册，
// 与用户插件同表校验（namespace/wire 唯一、CallerPolicy）。

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

impl KernelPlugin for MemoryPlugin {
    fn info() -> Info {
        Info {
            namespace: "memory".into(),
            provides: vec![ServiceId::Memory],
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

    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> {
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

pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<MemoryPlugin>()
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
mod tests;
