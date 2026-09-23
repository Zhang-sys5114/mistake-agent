# 0045 — 统一 DeepSeek 单模型（退役视觉端点，图片走 Responses）

日期：2026-09-23
状态：已采纳
取代：ADR-0019（双端点模型方案）
修订：ADR-0020（Responses API「不支持图片输入」结论已过时）、ADR-0027（`LiveSettingsModelService` 不再按 `ModelKind` 重建）、ADR-0031（余额查询去掉 SiliconFlow 分支）、ADR-0014（ModelHandle 不再按用途选模型）

## 背景

旧方案（ADR-0019/0020）是双模型双端点：主模型 `deepseek-v4-flash` 走 DeepSeek Responses API 负责对话与调度；视觉模型 `qwen3-VL`（硅基流动 SiliconFlow）走 Chat Completions 负责 OCR / 图片理解。之所以拆两个端点，唯一理由是当时 DeepSeek Responses API 不支持图片输入。

2026-09 官方文档已更新：模型名收敛为 `deepseek-flash`（V4.1-Flash，旧名 `deepseek-v4-flash` / `deepseek-v4-flash-vision-exp` 仍接受但已退役），**Responses API 原生支持图片输入**——`message` 项可带 `input_image` content part（`image_url` 支持 `http(s)` URL 与 base64 data URL，`file_id` 走 Files API），`function_call_output` / `custom_tool_call_output` 也可带图片。图片限制与 Chat Completions 共用。

双端点的代价随之失去意义：用户要维护第二份 key、第二套模型 ID；设置页 / OOBE 多一步；余额卡片多查一个账户；`ModelKind` 选路、`RoutingModelService`、`build_vision_service` 都是为"按用途选模型"而生的间接层。单份 DeepSeek 配置即可同时承担**主对话 + 调度/摘要 + 图片理解**。

## 决策

**一份 DeepSeek 配置（`api_url` / `api_key` / `model` / `transport`）承担全部模型调用；不再有视觉端点。**

1. 默认模型 ID 由 `deepseek-v4-flash` 改为 `deepseek-flash`；`api_url` 仍为 `https://api.deepseek.com`，主链路仍默认 `transport=responses`。
2. **删除 `ModelKind` 与 `ModelRequest.model`**：模型契约不再有"按用途选模型"入口，`ModelService` 单实现，`RoutingModelService` 删除，`LiveSettingsModelService` 去掉 `kind` 字段、只重建一个适配器。
3. `vision::read`（图片理解）改走同一条 Responses 链路：`Message` 的用户附件在 `messages_to_responses_input` 中展开为 `input_image`（`data:{mime};base64,{data}`，`detail=high`），与 `messages_to_cc` 的 `image_url` 行为对齐。
4. `settings.json` 的 `vision_model` 字段**保留但不再使用**（ADR 级取舍）：结构体与 patch 仍解析该字段以免旧配置报错，但 kernel、余额、模型构建一律不读它；前端不再展示。
5. 余额查询只查 DeepSeek（`GET {base}/user/balance`）；`BalanceReport` 只含 `main`，`AuditRecord::BalanceChecked { ok }`。
6. 设置页删「视觉模型（OCR / 图片理解）」卡片与 SiliconFlow 余额项；OOBE 由四步收敛为三步（欢迎 → DeepSeek 模型 → 完成）；模型就绪判定只看 `main_model.key_set`。
7. `test_connection` 忽略历史 `model:"vision"` 参数，一律测 DeepSeek 配置，wire 不变。

## 影响

- **图片理解能力不变**：`deepseek-flash` 原生理解图片，OCR 提示词（`vision_prompt`）与判分管线不动；`vision::read` 工具名、CallerPolicy、附件暂存链（ADR-0042 TmpIo）全部不变。
- **数据兼容**：旧 `settings.json`（含 `vision_model` 的 SiliconFlow 配置）解析不报错，字段被静默忽略；新保存的 settings.json 仍带该字段（保留字段策略），值为默认空配置。用户无需迁移。
- **失去的能力**：不再支持"主模型与视觉模型分离计费/分离供应商"，也不再支持 `qwen3-VL` 这类第三方视觉端点。这是本次明确接受的收敛代价。
- **文档同步**：`PROJECT.md`（§3/§6/§7/§13 Responses 表 / 术语）、`CONTEXT.md`（Main model / Vision model / ModelHandle）、`docs/api.md`、`docs/usage.md`、`docs/testing.md`、`docs/requirements.md`、`docs/prompts.md`、`docs/kernel-dev.md`、`docs/TODO.md`，以及被修订的 ADR 0019 / 0020 / 0027 / 0031 / 0014。
- 回归验证：`cargo test`、`cargo clippy -- -D warnings`、`cd web && npm run build`；真实链路 `cargo test --test live_api -- --ignored`（图片 OCR + 判分归档应仍通过）。
