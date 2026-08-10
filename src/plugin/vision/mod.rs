//! vision 插件：通用图片理解（读图不判分不归档）。
//!
//! 上传图片/PDF 后，模型先调 `vision::read` 理解内容（作业/试卷转写文字，
//! 角色/照片等描述内容），再按用户意图决定：直接讲解/描述，或调 `grading::upload` 判分归档。
//! 插件信息：namespace = vision，requires = [Model, Storage]；tools = [read]
//! （Storage 用于附件暂存读取——ADR-0042 磁盘 IO 铁律：用户插件不持有文件句柄）

use std::sync::Arc;

use serde_json::Value;

use crate::kernel::agent::dispatch::ToolCallContext;
use crate::kernel::context::PluginContext;
use crate::kernel::contract::{CallerPolicy, Info, PluginError, ToolDef};
use crate::kernel::plugin::services::ServiceId;
use crate::kernel::registry::{PluginDescriptor, UserPlugin};

mod core;

use core::ReadParams;

pub struct VisionPlugin;

impl UserPlugin for VisionPlugin {
    fn info() -> Info {
        Info {
            namespace: "vision".into(),
            requires: vec![ServiceId::Model, ServiceId::Storage],
            tools: vec![ToolDef {
                name: "read".into(),
                user_visible: true,
                title: Some("看图读题".into()),
                group: Some("学习".into()),
                description:
                    "查看图片/PDF 的内容：作业/试卷会转写文字，其它图片（角色、照片等）会描述内容，只读不判分、不归档。上传文件后先调用本工具理解内容，再根据内容与用户意图决定下一步：讲解/回答、或调用 grading::upload 判分归档。file 参数使用消息里给出的暂存路径。"
                        .into(),
                params: schemars::schema_for!(ReadParams),
                policy: CallerPolicy::UserAndModel,
                timeout: Some(180),
                icon: Some("mdi:image-search".into()),
            }],
            ..Default::default()
        }
    }

    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError> {
        let model = ctx
            .handles
            .model()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Model 句柄".into()))?;
        let storage = ctx
            .handles
            .storage()
            .cloned()
            .ok_or_else(|| PluginError::Internal("缺少 Storage 句柄".into()))?;
        ctx.registrar.tool(
            "read",
            Arc::new(move |call_ctx: &ToolCallContext, params: Value| {
                let model = model.clone();
                let storage = storage.clone();
                Box::pin(async move { core::read_handler(call_ctx, params, model, storage).await })
            }),
        )
    }
}

pub fn descriptor() -> PluginDescriptor {
    PluginDescriptor::from_plugin::<VisionPlugin>()
}

/// 供 grading::upload 复用：读图/PDF → 图片理解文本（pub(crate) 插件内共享）。
pub(crate) use core::{map_model_error, read_content};
