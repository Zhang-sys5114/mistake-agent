# Mistake Agent v2 — API 文档（standalone 单文件）

> 本文档自包含：GUI ↔ kernel 的 RPC 协议、内核入口点/服务契约、真实模型 API 对接方式与验收命令。代码引用均为仓库内文件名，细节以代码为准（ADR 见 docs/adr/）。

## 1. 架构总览

两个进程，一根 stdio 管道；UI 为 Vue 3（web/src，Vite 构建到 web/dist 由 Tauri 嵌入加载）：

```
Tauri GUI（web/ 静态 UI，src/main.rs 桥接）
        │  stdout 逐行 JSON 帧（Event 流）
        ▼
   sidecar（src/bin/sidecar.rs，kernel 进程）
```

- GUI → kernel：stdin 逐行写**请求帧**（`RpcRequest`，见 src/kernel/rpc.rs）。
- kernel → GUI：stdout 逐行写**响应帧**（带 id 回执）与**事件帧**（无 id 播报，`Event`，见 src/kernel/events.rs）。
- Tauri 侧桥接（src/main.rs 的 Tauri 命令，非 RPC 方法）：`start_kernel`（拉起 sidecar，stdout 逐行经 `Channel<String>` 推给前端）、`kernel_send`（写一行 JSONL 到 stdin）、`pick_homework_file`（rfd 文件对话框，返回作业路径）。
- 通信格式：JSON Lines（每行一个完整 JSON 对象），UTF-8。

## 2. 帧格式（GUI ↔ kernel）

### 2.1 请求帧（GUI → kernel）

```json
{"id": 1, "method": "send_user_message", "text": "你好"}
```

`RpcRequest { id: u64, #[serde(flatten)] method: Method }`——方法参数平铺在顶层，没有 `params` 包装。

| method | 参数 | 状态 | 说明 |
|---|---|---|---|
| `send_user_message` | `text: string` | ✅ M1 | 开新回合；若已有回合在跑返回错误 `turn_in_progress`（先停止再发送） |
| `trigger_command` | `entry: string`, `params: object` | ✅ M1 | 唯一命令通道，校验 EntryPoint + CallerPolicy |
| `abort` | — | ✅ M1 | 停止当前回合（SIGTERM → 宽限 → SIGKILL） |
| `get_state` | — | ✅ M1 | 返回 `{status: idle\|busy, session_key}` |
| `edit_message` | `message_id`, `text` | 🔒 M5 | 消息树编辑 |
| `switch_branch` | `message_id` | 🔒 M5 | 消息树切分支 |
| `get_settings` / `set_settings` | — | 🔒 M2/M5 | 设置向导 |

> Tauri 侧命令（GUI 专属，见 src/main.rs）：`start_kernel`、`kernel_send`、`pick_homework_file`；前端经 `@tauri-apps/api` 的 `invoke` 调用（web/src/composables/useKernel.js）。

### 2.2 响应帧（kernel → GUI）

```json
{"type":"response","id":1,"result":{"accepted":true}}
{"type":"response","id":1,"error":{"code":"turn_in_progress","message":"当前有回合在跑，请先停止再发送新消息"}}
```

`result` 与 `error` 二选一。错误码：`turn_in_progress` / `scheduler_error` / `tool_error` / `not_implemented`。

### 2.3 事件帧（kernel → GUI，无 id）

```json
{"type":"event","event":{"event":"message_delta","message_id":"...","delta":"你"}}
```

| event | 负载 | 说明 |
|---|---|---|
| `message_delta` | `message_id`, `delta` | 打字机增量（气泡 = 一个输出 item，完成即落盘） |
| `reasoning_delta` | `delta` | 思维链增量（UI 默认折叠，点击展开） |
| `tool_start` / `tool_end` | `entry`, `ok?` | 工具生命周期 |
| `tool_progress` | `entry`, `message` | 长任务进度（如"正在识别第 3/12 页"） |
| `turn_end` | `stop_reason` | `natural` / `tool_call_limit` / `consecutive_failures` / `turn_timeout` / `user_aborted` / `internal_abort` |
| `session_switched` | `from`, `to` | 会话切换（内部键，UI 不展示） |
| `memory_changed` | `path` | 记忆变更 |
| `compaction` | `session` | 上下文压缩 |
| `error` | `message` | 错误播报 |

## 3. 内核入口点契约

### 3.1 两段式插件契约（src/kernel/context.rs, registry.rs）

```rust
pub trait UserPlugin {
    fn info() -> Info;                                        // 静态元数据
    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError>; // 绑定 handler
}
```

- `Info`：`namespace`（全局唯一）、`requires`（能力声明）、`load`（eager/lazy，默认 lazy）、`tools`/`commands`/`events`。
- `ToolDef`：短名 + 描述 + `Schema`（schemars）+ `CallerPolicy`（`user_and_model` / `user_only`）+ 可选 `timeout`（秒）。
- 启动时 fail-fast 校验：namespace 唯一、全名跨 kind 唯一、wire name 全局唯一、requires 可满足、CallerPolicy 合法。
- lazy 插件首次命中任一入口时才执行 `register`；`EntryRegistrar` 只允许登记 info 声明过的短名。

### 3.2 命名：内部全名 vs wire name

- 内部全名：`namespace::tool`（如 `grading::upload`），用于注册表、审计、`trigger_command`。
- 模型可见名（wire name）：`::` → `_`（`grading_upload`），因为 Responses API 要求函数名匹配 `^[a-zA-Z0-9_-]+$`。注册时校验 wire name 全局唯一保证一一对应，模型回包经 dispatch 映射回全名（src/kernel/contract.rs `full_to_wire`，dispatch.rs `resolve_wire`）。

### 3.3 当前入口点

| 全名 | 类型 | 策略 | 说明 |
|---|---|---|---|
| `demo::hello` | tool | user_and_model | 链路自检 |
| `grading::upload` | tool | user_and_model | 场景一：`{file: 路径}` 图片(png/jpg/jpeg/webp/bmp)或文本型 PDF |
| `grading::list` | tool | user_and_model | `{subject?, knowledge_point?}` 列出错题本 |

## 4. 服务契约（src/kernel/services.rs）

| 服务 | 角色 trait | 注入视图 | 说明 |
|---|---|---|---|
| Storage | `SessionStore` + `MistakeStore` + `AuditSink` | `StorageHandle`（只有错题本 5 操作） | 会话/错题/审计；M1 内存实现（storage.rs） |
| Memory | `MemoryService`（save/show/remove，remove 删子树） | `MemoryHandle` | 路径类型化校验（memory.rs / services.rs） |
| Compute | `ComputeService::run` | `ComputeHandle` | M1 执行端（GUI Pyodide）未接，返回 `BackendUnavailable` |
| Model | `ModelService::stream/complete` | `ModelHandle`（仅 complete + 超时/abort/审计） | 路由主/视觉模型（model.rs） |

## 5. 真实模型 API 对接

### 5.1 主模型：DeepSeek Responses API（第一方，ADR-0020）

- Endpoint：`POST https://api.deepseek.com/responses`（无状态：每次请求全量历史，不支持 `previous_response_id`/`conversation`/`store`）。
- 模型：`deepseek-v4-flash`（2026-08 起官方支持；v4-pro 待官方放开）。
- 流式：语义 SSE 事件（`event:`/`data:` 行，空行分隔），结束事件 `response.completed` / `response.incomplete` / `response.failed`，**没有 `data: [DONE]`**（src/kernel/model.rs `SseParser`）。
- 事件映射：`output_text.delta`→TextDelta、`reasoning_text.delta`→ReasoningDelta、`function_call_arguments.delta`→ToolCallDelta、`output_item.done`→ItemDone（气泡/工具调用边界）、`response.completed`→Usage+Done。
- JSON 严格要求：`text.format` 支持 `json_object` 与 `json_schema`（判分用 json_schema 数组，schema 必须内联扁平、避免 `$defs/$ref`，DeepSeek 端不解析引用）。
- 思考模式默认开启：`reasoning.effort` 可传 `none`（判分用 none 提速）；thinking 下 temperature/top_p 无效。
- 工具：function 名约束 `^[a-zA-Z0-9_-]+$`（wire name）；`parallel_tool_calls` 恒开启（参数被忽略），loop 串行执行。
- 传输兜底：客户端强制 IPv4 本地地址（无 IPv6 环境稳定连通）。

### 5.2 视觉模型：SiliconFlow Chat Completions（仅 OCR，不判分）

- Endpoint：`POST https://api.siliconflow.cn/v1/chat/completions`。
- 模型：`Qwen/Qwen3-VL-32B-Instruct`（settings 可配 `SILICONFLOW_MODEL`）。
- 图片：`content` 数组 `{"type":"image_url","image_url":{"url":"data:<mime>;base64,...","detail":"high"}}` + `{"type":"text","text":"仅转写，不要解题"}`（src/plugin/grading.rs `ocr_image`）。
- PDF：文本型 PDF 用 `pdf-extract` 提取文字；扫描版 PDF 明确报错提示拍照上传。

### 5.3 settings.json（数据根目录 `~/Documents/.mistake-agent/`）

```json
{
  "log_level": "info",
  "main_model": { "api_url": "https://api.deepseek.com", "api_key": "...", "model": "deepseek-v4-flash", "transport": "responses" },
  "vision_model": { "api_url": "https://api.siliconflow.cn/v1", "api_key": "...", "model": "Qwen/Qwen3-VL-32B-Instruct" }
}
```

环境变量回退：`DEEPSEEK_API_KEY` / `DEEPSEEK_API_URL` / `SILICONFLOW_API_KEY` / `SILICONFLOW_API_URL` / `SILICONFLOW_MODEL` / `MISTAKE_AGENT_LOG_LEVEL`。

## 6. 超时与取消模型（ADR-0022）

- 两级取消：SIGTERM（合作式，handler 自主收尾，宽限 5s）→ SIGKILL（dispatch 掐任务）。
- 三层超时：工具级（ToolDef.timeout，默认 30s）< 回合级（10min）< 活性超时（流式 60s 无增量断）。
- 延期后门：`DeadlineHandle::extend`，受回合预算钳制 + 审计。
- OCR 页级失败：重试 2 次 → 页级错误记结果继续；系统性模型错误直接 `ToolError::model_unavailable` 撂挑子。

## 7. 会话与消息树（ADR-0006/0007）

- `SessionKey` = UUID；守卫模型（M1.5 stub 关键词版，生产=M2 小模型调用）在"新消息到达"时决策 continue/update_goal/start_new；**start_new 只在有新消息时触发**，回合结束只允许 continue/update_goal。
- 空闲超时 12h：超时后新消息直接开新会话（旧会话归档 + 交接摘要注入）。
- 消息气泡：一个输出 item = 一个气泡，**完成即落盘**；中断只丢半截，已完整气泡保留。
- `InterruptBus`（内部中断，ADR-0023）：环境变更信号队列，agent loop 回合边界消费。

## 8. 运行与验收

```bash
cd web && npm install && npm run build    # 前端构建（改过 web/ 后必须执行）
cargo test                                # 单元测试（14 项）
cargo test --test live_api -- --ignored   # 真实 API 验收：hello + samples/ 三套样例
cargo run --bin sidecar                   # kernel CLI：管道喂 JSONL
cargo run --bin mistake-agent             # Tauri GUI（Wayland/X11 均可）
```

sidecar 管道示例：

```bash
printf '%s\n' '{"id":1,"method":"send_user_message","text":"你好"}' | cargo run --bin sidecar
```

门禁：`cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`。

## 9. 代码文件索引

| 文件 | 内容 |
|---|---|
| src/kernel/contract.rs | 入口点元数据、CallerPolicy、ToolError、wire name |
| src/kernel/services.rs | 四服务契约、受控句柄、ServiceHandles、MemoryPath |
| src/kernel/model.rs | Responses API / Chat Completions 适配器、SSE 解析、路由服务 |
| src/kernel/registry.rs / context.rs | 注册表校验、两段式契约、EntryRegistrar |
| src/kernel/dispatch.rs | Caller 检查、jsonschema 校验、两级取消、延期后门 |
| src/kernel/loop_mod.rs | agent loop、护栏、气泡完成落盘 |
| src/kernel/session.rs | SessionScheduler、守卫模型、InterruptBus、空闲超时 |
| src/kernel/storage.rs / memory.rs / compute.rs | 服务实现（M1 内存） |
| src/kernel/rpc.rs | 帧类型、Kernel 组装与请求路由 |
| src/bin/sidecar.rs | stdio JSONL 主循环 |
| src/main.rs | Tauri 壳：拉起 sidecar + Channel 桥接 |
| web/ | Vue 3 UI（src/App.vue、composables/useKernel.js，构建产物 web/dist） |
| src/plugin/grading.rs | 场景一：上传/OCR/判分/归档 |
| tests/live_api.rs | 真实 API 验收测试 |
| samples/ | 三套作业样例（1 真实照片 + 2 合成卷） |
