# 0046 — 图片直入上下文、grading 只归档（退役 vision::read）

日期：2026-09-23
状态：已采纳
取代：ADR-0019 的视觉链路实现（视觉端点已于 ADR-0045 退役）
修订：ADR-0007（用户消息新增路径式附件引用）、ADR-0042（附件暂存 TmpIo 不再被业务消费）、ADR-0038（`grading::upload` 入参由文件路径改为判分结果）

## 背景

ADR-0045 之后，主模型 `deepseek-flash` 的 Responses API 原生支持图片输入。但旧链路仍是两步：模型先调 `vision::read(file)` 让模型转写/描述图片（一次模型调用），再根据文本决定是否调 `grading::upload(file)`——而 `grading::upload` 内部又自己读图 + 调一次模型判分。同一张图被模型看了两到三次，中间产物是文字。

既然主模型能直接看图，这层「图片 → 文字 → 再判分」的中间环节就是多余的：模型可以直接在对话上下文里阅读用户上传的图片，判分也由同一个模型完成，工具只需把结构化结果落库。

## 决策

**图片直接进模型上下文；`vision::read` 退役；`grading::upload` 改为只归档模型判分结果。**

1. **附件以路径引用持久化**：用户消息新增 `attachment_refs: Vec<AttachmentRef>`（`uploads/` 域内相对文件名 + mime + 原名），消息树只存引用不存字节；运行时附件字段 `attachments` 标 `#[serde(skip)]`，不落盘。
2. **请求构建时解析**：新增 `AttachmentResolvingModelService` 包装层，在每次模型请求前按 `attachment_refs` 读 `uploads/` 文件、base64 回填 `attachments`（进程内按文件名缓存，避免每回合重复磁盘 IO 与审计噪声）；Responses 适配器再把附件展开为 `input_image`（ADR-0045 已实现）。loop 与 `ModelHandle` 均使用该包装层。
3. **删除 `vision::read` 工具与 `vision` 插件**：不再有独立的「读图」步骤；`map_model_error` 辅助迁入 `practice`。
4. **`grading::upload` 改为归档**：入参由 `{file}` 变为 `{items: Vec<GradedItem>}`（内联 JSON Schema，避免 `$defs/$ref`）。模型直接阅读上下文中的图片/PDF 正文，逐题填写判分字段后调用该工具；工具只把 `correct=false` 的题落库。`requires` 缩减为 `[Storage]`，不再持有 Model 句柄。
5. **PDF**：Responses API 不支持文件输入，PDF 在 GUI 边界（`stage_files`）用 `pdf-extract` 抽取正文，作为消息文本随消息携带（`SendUserMessage.display_text` 保留用户原文用于展示）；不再创建系统临时暂存副本（TmpIo 附件通道随之闲置，代码保留但业务不再调用）。
6. **提示词**：`vision_prompt` / `grading_system_prompt` 退役，批改规则（LaTeX/SMILES、语法判分、解答题给分）并入常驻系统提示 `GRADING_GUIDANCE`；主系统提示的作业流程改写为「直接阅读消息里的图片/PDF 正文 → 决定讲解或调用 `grading::upload` 归档」。
7. **RPC**：`SendUserMessage` 新增 `display_text`（展示与模型文本分离）；`file` 字段保留但弃用；`asset` 用于构造图片引用。

## 依据

DeepSeek 官方文档（2026-09-23 核对）：Responses API 的 `message` 项支持 `input_image` content part（base64 data URL 或 http(s) URL），`deepseek-flash` 将其作为真实图片处理；文件输入不支持，故 PDF 仍需抽文。

## 影响

- **成本取舍**：图片进入消息树后随无状态 API 每回合重发（上下文缓存可缓解），替代了原先「读图一次转文字」的低成本路径。这是本次明确接受的代价，换来的是链路简化与「所见即所判」。
- **数据兼容**：旧会话消息没有 `attachment_refs`，前端回退解析文本里的「附件：路径|名称」标记；旧 `attachments` 字段不再序列化，反序列化时忽略。
- **能力变化**：不再有独立的「图片理解」工具调用步骤；`vision::read` 从工具目录消失（`list_tools` 自动同步）。`grading::upload` 的 UserOnly/UserAndModel 策略与名称不变，`trigger_command` 兼容。
- **未验证项**：`grading::upload` 以工具参数承载 `items` 数组（含题面长文本）依赖模型稳定产出合法工具参数；需 `cargo test --test live_api -- --ignored` 真实链路复验。
- **文档同步**：`PROJECT.md`、`CONTEXT.md`、`docs/api.md`、`docs/prompts.md`、`docs/usage.md`、`docs/requirements.md`、`docs/testing.md`、`docs/kernel-dev.md`、`docs/plugin-dev/user.md`、`docs/TODO.md`、`README.md`、`CHANGELOG.md`，以及被修订的 ADR 0007 / 0038 / 0042。
