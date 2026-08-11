//! grading 插件（任务三·场景一）：上传作业 → 读图（vision::read）→ 判分 → 错题归档。
//!
//! 插件信息：namespace = grading，requires = [Storage, Model]
//! tools = [upload（判分归档）, list（错题本）]；看图（vision::read）独立成 vision 插件。
//! 实现拆分（Linux 内核风格）：`params.rs` 参数与结果 schema / `core.rs` 判分·归档 handler

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, CommandDef, Info, PluginError, ToolDef, ToolError};
use crate::kernel::plugin::services::MistakeFilter;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};
mod core;
mod params;

use core::{
    get_handler, remove_handler, remove_many_handler, update_handler, upload_handler,
};
use params::{GetParams, ListParams, RemoveManyParams, RemoveParams, UpdateParams, UploadParams};

pub struct GradingPlugin;

impl UserPlugin for GradingPlugin {
    fn info() -> Info {
        Info {
            namespace: "grading".into(),
            // 默认懒加载：工具列表（model_tools）读 info 声明，第一轮即可见；
            // 模型 wire 调用命中未加载插件时由 resolve_wire 触发懒加载（ADR-0003）。
            requires: vec![
                crate::kernel::plugin::services::ServiceId::Storage,
                crate::kernel::plugin::services::ServiceId::Model,
            ],
            tools: vec![
                ToolDef {
                    name: "upload".into(),
                    user_visible: true,
                    title: Some("上传作业批改".into()),
                    group: Some("批改".into()),
                    description:
                        "上传作业图片或文本型 PDF，自动识别题目与作答、判分、把错题归档进错题本。文件由应用「选择作业文件」按钮上传后自动暂存，file 参数使用消息里给出的暂存路径。"
                            .into(),
                    params: schemars::schema_for!(UploadParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: Some(180),
                    icon: Some("mdi:upload".into()),
                },
                ToolDef {
                    name: "list".into(),
                    user_visible: true,
                    title: Some("查看错题本".into()),
                    group: Some("批改".into()),
                    description: "列出错题本中的错题（可按学科/知识点过滤）。用法：grading::list [学科] [知识点]".into(),
                    params: schemars::schema_for!(ListParams),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: Some("mdi:format-list-bulleted".into()),
                },
            ],
            commands: vec![
                CommandDef {
                    name: "get".into(),
                    user_visible: false,
                    title: Some("查看错题详情".into()),
                    group: Some("错题本".into()),
                    description: "按 id 获取单条错题详情，供详情页和追问使用。".into(),
                    params: schemars::schema_for!(GetParams),
                    icon: Some("mdi:card-text-outline".into()),
                },
                CommandDef {
                    name: "update".into(),
                    user_visible: false,
                    title: Some("编辑错题".into()),
                    group: Some("错题本".into()),
                    description: "编辑错题字段；置顶传 pinned，标记已掌握传 is_correct=true。".into(),
                    params: schemars::schema_for!(UpdateParams),
                    icon: Some("mdi:pencil-outline".into()),
                },
                CommandDef {
                    name: "remove".into(),
                    user_visible: false,
                    title: Some("删除错题".into()),
                    group: Some("错题本".into()),
                    description: "软删除单条错题，列表不再展示，数据仍保留。".into(),
                    params: schemars::schema_for!(RemoveParams),
                    icon: Some("mdi:delete-outline".into()),
                },
                CommandDef {
                    name: "remove_many".into(),
                    user_visible: false,
                    title: Some("批量删除错题".into()),
                    group: Some("错题本".into()),
                    description: "按 id 列表批量软删除，支持全选后整页删除。".into(),
                    params: schemars::schema_for!(RemoveManyParams),
                    icon: Some("mdi:delete-sweep-outline".into()),
                },
            ],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let storage = ctx
            .handles
            .storage()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Storage 句柄".into()))?;
        let model = ctx
            .handles
            .model()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Model 句柄".into()))?;

        let storage_upload = storage.clone();
        ctx.registrar.tool(
            "upload",
            std::sync::Arc::new(move |call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_upload.clone();
                let model = model.clone();
                Box::pin(async move { upload_handler(call_ctx, params, storage, model).await })
            }),
        )?;

        let storage_list = storage.clone();
        ctx.registrar.tool(
            "list",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_list.clone();
                Box::pin(async move {
                    let filter: ListParams = serde_json::from_value(params)
                        .map_err(|e| ToolError::invalid_params(e.to_string()))?;
                    let filter = MistakeFilter {
                        subject: filter.subject,
                        knowledge_point: filter.knowledge_point,
                        is_correct: None,
                    };
                    let mistakes = storage
                        .list(&filter)
                        .await
                        .map_err(|e| ToolError::handler(e.to_string()))?;
                    Ok(json!({
                        "count": mistakes.len(),
                        "mistakes": mistakes,
                    }))
                })
            }),
        )?;

        let storage_get = storage.clone();
        ctx.registrar.command(
            "get",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_get.clone();
                Box::pin(async move { get_handler(storage, params).await })
            }),
        )?;

        let storage_update = storage.clone();
        ctx.registrar.command(
            "update",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_update.clone();
                Box::pin(async move { update_handler(storage, params).await })
            }),
        )?;

        let storage_remove = storage.clone();
        ctx.registrar.command(
            "remove",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_remove.clone();
                Box::pin(async move { remove_handler(storage, params).await })
            }),
        )?;

        let storage_remove_many = storage.clone();
        ctx.registrar.command(
            "remove_many",
            std::sync::Arc::new(move |_call_ctx: &ToolCallContext, params: Value| {
                let storage = storage_remove_many.clone();
                Box::pin(async move { remove_many_handler(storage, params).await })
            }),
        )?;

        Ok(())
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<GradingPlugin>()
}


#[cfg(test)]
mod tests;
