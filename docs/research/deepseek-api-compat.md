# DeepSeek 官方 API 能力边界调研（面向「带鉴权 + 计费」的中转网关）

> 研究时间：2026-09-30。目标：为 `server/`（ADR-0047 的 DeepSeek 中转 + 售卖）确定上游 API 的
> 真实能力边界——尤其 **Anthropic 兼容端点是否存在**、各端点的 usage 形状、错误语义与计价。
>
> **证据规则**：只采信 DeepSeek 官方文档（`api-docs.deepseek.com`）与官方站点上的其他一手页面；
> 官方未记载的项一律写「官方文档未记载」，不用二手博客或推测填空。
> 本机 `hosts` 无关（仅 Steam++ 屏蔽项）；文档站解析到代理地址 `198.18.0.168`，`web_fetch` 会拒绝该非公网 IP，
> 因此正文引用的原文均通过本机 HTTPS 直连抓取（见 §10 核对方法）。

## 1. 结论先行

1. **Anthropic 兼容端点存在且是一等公民**：base_url = `https://api.deepseek.com/anthropic`，
   路径即 Anthropic Messages API 的 `/v1/messages`（SDK 自动拼接）。鉴权用 `x-api-key`（完全支持），
   `anthropic-version` 头被**忽略**、不需要。[S6][S7]
2. **Anthropic 形状的请求：请求侧可原样转发，响应侧不能假设形状**。请求侧字段层面高度兼容
   （`tools` / `tool_use` / `tool_result` 完全支持），但 **Anthropic 端点的 SSE 事件名与 usage
   官方文档均未记载**，且 `cache_control` 被忽略、`container` / `mcp_servers` / `top_k` /
   `service_tier` 也被忽略。[S6] 网关若同时服务 OpenAI 与 Anthropic 两类客户端，
   **响应侧（尤其事件名与 usage 字段）必须按端点分别处理或做双向翻译**。
3. **Responses API 真实存在**，`POST https://api.deepseek.com/responses`，无状态
   （`store` / `previous_response_id` / `conversation` 全不支持），流式结束事件为
   `response.completed` / `response.incomplete` / `response.failed`，**没有 `data: [DONE]`**；
   而 Chat Completions **有** `data: [DONE]` 收尾（官方原文），Anthropic 端点的收尾标记
   DeepSeek 文档未写（按 Anthropic 原生规范应是 `message_stop`，须实测）。[S10][S11][S8][S5][S6]
4. **计费数据完全够用**：Responses 与 Chat Completions 的 usage 都区分缓存命中 / 未命中；
   计价按「高峰 / 空闲」两档，空闲价为高峰价的一半。[S11][S5][S2]
5. 仓库现有假设**基本全部与官方文档一致**，但有 3 处需要修正（§8）：
   ①「2026-08 起支持 Responses」更准确是 **2026-07-31（Flash）/ 2026-08-13（Pro）**；
   ② 旧模型名 `deepseek-v4-flash` 不是"退役报错"而是**仍可调用并被路由到 V4.1-Flash**；
   ③ `deepseek-v4-pro` **不支持图片输入**，图片能力只在 `deepseek-flash` 上。[S12][S2][S13]

## 2. Anthropic 兼容端点（重点）

### 2.1 存在性与 base URL

官方在《使用 Anthropic API》开篇写明：

> "为了满足大家对 Anthropic API 生态的使用需求，我们的 API 新增了对 Anthropic API 格式的支持，
> 其 base_url 为 `https://api.deepseek.com/anthropic`。"[S6]

官方配置示例：

```python
export ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
export ANTHROPIC_API_KEY=${YOUR_API_KEY}

client = anthropic.Anthropic()          # 读环境变量
message = client.messages.create(model="deepseek-flash", max_tokens=1000, ...)
```
[S6]

- **确切 base URL**：`https://api.deepseek.com/anthropic`。[S6]
- **确切路径**：官方文档只给出 base_url 与 SDK 用法；Anthropic Python SDK 在 base_url 后追加
  `/v1/messages`，故完整端点为 `POST https://api.deepseek.com/anthropic/v1/messages`。
  **官方文档未直接写出该完整路径**（未找到一手来源逐字确认），建议落地前实测一次。
- 同一个 base URL 也出现在《模型 & 价格》表里（"BASE URL (Anthropic 格式)"一栏）。[S2]

### 2.2 鉴权

| 头 | 官方支持情况 |
|---|---|
| `x-api-key` | **完全支持** |
| `anthropic-version` | **忽略**（不需要传，传了也不校验） |
| `anthropic-beta` | `/messages` 上忽略；仅 Files API 端点必须携带 `files-api-2025-04-14` |

[S6]

- 即：用 **`x-api-key`**，**不需要 `anthropic-version`**。
- 另有一条官方一手线索：Claude Code 接入文档用的是 `ANTHROPIC_AUTH_TOKEN` 环境变量（→ Claude Code
  以 `Authorization: Bearer` 形式发送）。[S7] 两种头在这一端点上都能工作的完整并集边界
  **官方文档未给出明确表述**；建议网关**同时接受 `x-api-key` 与 `Authorization: Bearer`**，
  并以实测确认。
- **OpenAI / Responses 面用 `Authorization: Bearer`**（官方 curl 示例）。[S1]
  **同一把 key 通吃**：文档从未按面区分 key，`api_key` 只有一个。[S1][S2]

### 2.3 模型与模型名映射

- 可用模型：`deepseek-flash`、`deepseek-v4-pro`。[S2][S13]
- 传入**不支持的模型名**时，后端自动映射到 `deepseek-flash`（不报错）。[S6]
- 传入 `claude-*` 名时的映射（面向 Claude Code / Claude Desktop developer 模式）：

| 传入 | 映射到 | 计价 |
|---|---|---|
| `claude-opus*` | `deepseek-v4-pro` | 按 V4 Pro 价 |
| `claude-haiku*` / `claude-sonnet*` | `deepseek-flash` | 按 Flash 价 |

[S6][S7]

- `GET /models` 还暴露了 `api_capabilities.anthropic_messages.system_prompt_update`：
  `deepseek-flash` 为 `in-history`（可在对话历史里追加 system 消息、最新一条覆盖此前全部），
  `deepseek-v4-pro` 为 `leading-only`（只有开头那条 system 生效）。[S13]

### 2.4 流式与 SSE 事件名

- `stream`：**完全支持**。[S6]
- **SSE 事件名清单：官方文档未记载**。DeepSeek 文档没有给出 Anthropic 端点的事件名段落，
  只说"Anthropic API 完整格式定义，请参考 Anthropic 官方 API 手册"。[S6]
  因此事件名应视为 **Anthropic 原生 Messages 流规范**（`message_start` / `content_block_start` /
  `content_block_delta` / `content_block_stop` / `message_delta` / `message_stop` 等），
  但**这是一手来源之外的推断，须实测核对**。
- 已知的一手保活行为：流式请求会持续收到 SSE 注释 `: keep-alive`；10 分钟未开始推理则服务端关连接。[S4]

### 2.5 Tool use

| 字段 | 支持情况 |
|---|---|
| `tools[].name` / `input_schema` / `description` | 完全支持 |
| `tools[].cache_control` | 忽略 |
| `tool_choice: none` | 完全支持 |
| `tool_choice: auto` / `any` / `tool` | 支持（`disable_parallel_tool_use` 被忽略 → 恒并行） |
| 消息块 `tool_use`（`id` / `input` / `name`） | 完全支持（`cache_control` 忽略） |
| 消息块 `tool_result`（`tool_use_id` / `content`） | 完全支持（`cache_control`、`is_error` 忽略） |

[S6]

另支持 `server_tool_use` 与 `web_search_tool_result`（Claude Code 的 Web Search 由 DeepSeek 侧
原生支持，会额外产生模型 token 费用）；不支持 `document`、`search_result`、`redacted_thinking`、
`code_execution_tool_result`、`mcp_tool_use` / `mcp_tool_result`、`container_upload`。[S6][S7]

### 2.6 官方「把 Claude Code 接到 DeepSeek」文档

存在，标题《接入 Claude Code》，给出从零安装与从现有安装迁移两套环境变量：

```powershell
$env:ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic"
$env:ANTHROPIC_AUTH_TOKEN="<你的 DeepSeek API Key>"
$env:ANTHROPIC_MODEL="deepseek-flash[1m]"
$env:ANTHROPIC_DEFAULT_OPUS_MODEL="deepseek-flash[1m]"
$env:ANTHROPIC_DEFAULT_SONNET_MODEL="deepseek-flash[1m]"
$env:ANTHROPIC_DEFAULT_HAIKU_MODEL="deepseek-flash"
$env:CLAUDE_CODE_SUBAGENT_MODEL="deepseek-flash"
$env:CLAUDE_CODE_EFFORT_LEVEL="max"
$env:CLAUDE_CODE_AUTO_COMPACT_WINDOW="786432"
```
[S7]

注意 `deepseek-flash[1m]` 这种**带方括号后缀的写法**只出现在 Claude Code 的 env 示例里；
官方未解释该语法，也未计入常规模型名清单。[S7][S2]

### 2.7 结论：能否原样透传

| 面 | 结论 |
|---|---|
| 请求侧字段 | **基本可原样转发**。`model` / `max_tokens` / `stream` / `system` / `stop_sequences` / `temperature` / `thinking` / `output_config.effort` / `tools` / `tool_choice` / 消息块 `text` / `image` / `thinking` / `tool_use` / `tool_result` 均支持；`anthropic-version`、`top_k`、`container`、`mcp_servers`、`service_tier`、`cache_control` 被静默忽略。注意 `temperature` 范围 **[0.0 ~ 2.0]**（宽于 Anthropic 原生 0~1），`thinking.budget_tokens` 被忽略。[S6] |
| 响应侧 | **必须翻译或至少分端点处理**。① SSE 事件名与 OpenAI 形状完全不同（本报告 §2.4 无法用一手来源定死）；② Anthropic 端点的 usage 字段形状**官方文档未记载**（§5.3）；③ 收尾标记不同：Chat Completions 以 `data: [DONE]` 收尾，Responses **没有** `[DONE]`，Anthropic 端点官方未记载（按 Anthropic 原生规范应为 `message_stop`）。[S5][S10][S11][S6] |

**给设计的一句话**：Anthropic 面可以做成"请求直通 + 响应按 Anthropic 规范产出"的薄适配层，
但**计费字段必须自己从 SSE 里解析**，不能假设 DeepSeek 会给出 Anthropic 原生 usage（见 §5.3）。

## 3. OpenAI 兼容面（Chat Completions）

- **路径**：`POST /chat/completions`（官方 curl 完整写法 `https://api.deepseek.com/chat/completions`）。[S1][S5]
- **base_url 写法**：官方文档**一律写 `https://api.deepseek.com`**（OpenAI SDK 的 `base_url`
  与 curl 的完整 URL 都是这一种形态），文档正文**从未出现 `/v1`**。[S1][S2]
  - SDK 侧：`OpenAI(base_url="https://api.deepseek.com")`，SDK 自行拼 `/chat/completions`。[S1]
  - 用户实测 `https://api.deepseek.com/chat/completions` 返回 200；`base_url` 带 `/v1` 能否同样工作
    **官方文档未记载**（未找到一手来源），建议在网关里对 `/v1` 前缀做一次实测或直接兼容两种。
  - 仓库现有客户端 `responses_endpoint()` 会 strip 掉尾 `/v1`，这一做法与"官方只承诺不带 `/v1` 的形态"一致。
- **Responses 的 base_url 也是同一个**：`base_url = https://api.deepseek.com`（同一句
  "其 base_url 为 https://api.deepseek.com"）。[S11][S2]
- **同一把 key**：文档只有一个 `api_key`，OpenAI 面用 `Authorization: Bearer`，Anthropic 面用 `x-api-key`，
  未见按面区分密钥的说明。[S1][S2][S6]
- 认证模型：`deepseek-flash`、`deepseek-v4-pro`。[S2][S13]
- 流式：SSE，**以 `data: [DONE]` 结尾**；`stream_options.include_usage` 控制是否每块带 usage，
  且无论是否设置，`[DONE]` 前最后一个块都会带整轮统计（**不会单独下发只含 usage 的块**，
  统计挂在最后一个内容块上，该块 `choices` 只有一项、`finish_reason` 非 null、无新增内容）。[S5]

## 4. Responses API

### 4.1 存在性、路径、无状态

- 官方指南《使用 Responses API》开篇：为满足 Codex 需求新增对 Responses API 格式的支持，
  base_url 为 `https://api.deepseek.com`。[S10][S11]（另有官方《接入 Codex》配置页，同一 base_url。[S9]）
- 参考页：`POST /responses`。[S8]
- **无状态（明确写出）**：

> "The API is stateless: responses and conversations are not stored on the server.
> For multi-turn conversations, the client needs to send the full conversation history in `input` on each request."[S8]

  兼容性表逐条确认：`previous_response_id` **不支持**、`conversation` **不支持**、
  `store` **不支持（响应中恒为 `store: false`）**、`background` / `metadata` / `include` / `prompt` /
  `truncation` / `service_tier` / `safety_identifier` / `prompt_cache_key` / `prompt_cache_retention` /
  `context_management` / `stream_options` 均不支持。[S11]
  注意：**不支持的参数被静默忽略、不报错**，所以现有客户端无需改动即可接入。[S11]

### 4.2 SSE 事件名清单（官方完整列表）

| 事件 | 说明 |
|---|---|
| `response.created` | 首个事件；响应已创建，状态 `in_progress` |
| `response.in_progress` | 响应生成中 |
| `response.output_item.added` / `response.output_item.done` | 输出 item（reasoning / message / function_call / custom_tool_call）开始 / 完成 |
| `response.content_part.added` / `response.content_part.done` | item 内内容块开始 / 完成 |
| `response.reasoning_text.delta` / `response.reasoning_text.done` | 思维链增量 / 完整思维链 |
| `response.output_text.delta` / `response.output_text.done` | 输出文本增量 / 完整文本 |
| `response.function_call_arguments.delta` / `response.function_call_arguments.done` | Function 参数增量 / 完整参数 |
| `response.custom_tool_call_input.delta` / `response.custom_tool_call_input.done` | Custom 工具（apply_patch）输入增量 / 完整输入 |
| `response.completed` | 正常完成的最后一个事件，**携带含 usage 的完整 response 对象** |
| `response.incomplete` | 被截断（如达 `max_output_tokens`）的最后一个事件 |
| `response.failed` | 失败的最后一个事件，携带含 error 详情的完整 response 对象 |

[S10]

- 每个事件都带 `event` 字段（事件类型）与递增的 `sequence_number`。[S10][S8]
- **没有 `data: [DONE]`**。[S10][S8]
- `status` 枚举：`in_progress` / `completed` / `incomplete` / `failed`；
  `incomplete_details.reason` 为 `max_output_tokens` 或 `content_filter`；失败时 `error` 含 `code` 与 `message`。[S8]

### 4.3 usage 出现位置

- **非流式**：响应对象的顶层 `usage`。[S8]
- **流式**：在 `response.completed` / `response.incomplete` / `response.failed` 事件的
  `data.response.usage` 里（"response.completed ... 携带包含 usage 的完整 response 对象"）。[S10][S8]
- 也就是说：**计费只需盯住最后一个事件**——与 ADR-0047 的"SSE 边转发边旁路解析，只提取
  `response.completed` 的 usage"设计一致，但需把 `response.incomplete` / `response.failed` 也纳入
  （截断/失败的响应同样可能有已产生的 token）。[S10]

### 4.4 图片输入 `input_image`

- 仅 `deepseek-flash` 支持（`deepseek-v4-pro` 的 `input_modalities` 只有 `text`）。[S10][S13]
- 形式：`image_url`（**http(s) URL 或 base64 data URL**）或 `file_id`（Files API），两者互斥，
  都不传或都传均 400（错误串已给出：`input_image must have image_url or file_id` /
  `input_image cannot have both image_url and file_id`）。[S10][S8]
- `detail`：`low`（缩放到 512×512，更快更省）/ `high` / `original` / `auto`（后三者保留原图）；
  设置 `file_id` 时 `detail` 被忽略。[S10][S8]
- **位置限制**：只允许出现在 `message` 项的 user / developer（developer 视同 user）内容里，
  以及 `function_call_output` / `custom_tool_call_output` 的 `output` 里；
  **system / assistant 消息中的图片返回 400**。文件输入不支持。[S10][S8]
- 同一套图片限制照搬 Chat Completions：请求体 48 MiB、内联单张 32 MiB、Files API 单张 64 MiB、
  无 file_id 图片合计 64 MiB、含 file_id 最高 200 MiB、单请求图片数上限、每图最多 1024 token、
  单边 8192px（单请求 ≥15 张图时降为 4096px）；外部 http(s) URL 需 60 秒内下载完成。[S14]

### 4.5 thinking 与 `tool_choice` / `temperature`

- `reasoning.effort ∈ {none, low, high, max}`：`none` 关思考；`low/high/max` 开思考；
  **不设置时走模型默认（默认开启）**；兼容映射 `minimal→low`、`medium/xhigh→high`。[S8][S15]
- `temperature`：范围 ≤ 2，默认 1，**思考模式下不生效**。[S8]
- `top_p`：**仅思考模式生效，有效范围 0.95–1.0（低于 0.95 按 0.95 处理）；非思考模式恒为 1.0，传入被忽略**。[S8][S15]
- **`tool_choice` + 思考模式**：官方 **Chat Completions** 文档明确"思考模式下不支持 `required`
  和指定具体 tool，会返回 400，请先关闭思考模式"；**Responses API 文档没有这条限制说明**
  （其指南写 `tool_choice` 支持 `none / auto / required / 指定工具`）。[S5][S11]
  → 两处文档口径不一致，**Responses 面在 thinking 下的 `required` / 指定工具行为必须实测**。
- `parallel_tool_calls` 被忽略（**并行工具调用始终开启**）；`max_tool_calls` 被忽略。[S11]
- `text.format` 完整支持（`text` / `json_object` / `json_schema`）；`text.verbosity` 可传不生效；
  `reasoning.summary` 可传但不生成摘要。[S8][S11]
- Function 名约束：非空、**≤ 128 字符、匹配 `^[a-zA-Z0-9_-]+$`、全局唯一**。[S8]

## 5. usage 字段形状（计费口径）

### 5.1 Responses API

```json
"usage": {
  "input_tokens": 22,
  "input_tokens_details": { "cached_tokens": 0 },
  "output_tokens": 29,
  "output_tokens_details": { "reasoning_tokens": 27 },
  "total_tokens": 51
}
```
[S8]

- `input_tokens`（必填）、`output_tokens`（必填）、`total_tokens`（必填）。[S8]
- `input_tokens_details.cached_tokens` = **命中上下文缓存的输入 token 数**。[S8]
- `output_tokens_details.reasoning_tokens` = **思维链 token 数**。[S8]
- ⚠️ **未标 `required` 的字段（`input_tokens_details` / `output_tokens_details`）在流式下是否恒存在，
  官方未明确**；示例里 `response.completed` 的 usage 也是完整对象。[S8] 计费解析建议全部按可选处理。

### 5.2 Chat Completions

```json
"usage": {
  "completion_tokens": 10,
  "prompt_tokens": 16,
  "total_tokens": 26,
  "prompt_tokens_details": { "cached_tokens": 0 },
  "prompt_cache_hit_tokens": 0,
  "prompt_cache_miss_tokens": 16,
  "completion_tokens_details": { "reasoning_tokens": 0 }
}
```
[S5]

- `prompt_tokens` = `prompt_cache_hit_tokens` + `prompt_cache_miss_tokens`（官方等式）。[S5]
- `prompt_tokens_details.cached_tokens` **与 `prompt_cache_hit_tokens` 相同**（冗余字段）。[S5]
- `completion_tokens_details.reasoning_tokens` = 思维链 token 数。[S5]
- 缓存命中 / 未命中两字段的官方出处另见《上下文硬盘缓存》：
  "我们在 usage 字段中增加了两个字段……`prompt_cache_hit_tokens`、`prompt_cache_miss_tokens`"。[S16]

### 5.3 Anthropic 端点（重要缺口）

- **官方文档未记载该端点的 usage 字段形状**：整篇《使用 Anthropic API》没有 usage 段落，
  也没有请求/响应示例展示 usage（已逐行核对全文 102 行）。[S6]
- 因此**不能凭文档确认**它会返回 Anthropic 原生的
  `input_tokens` / `output_tokens` / `cache_read_input_tokens` / `cache_creation_input_tokens`。
- 另有两条只能算**弱线索**（非 DeepSeek 一手来源，不作为结论）：
  ① `GET /models` 明确把这一面称作 "Anthropic Messages API"，声称响应结构兼容 Anthropic 生态 [S6][S13]；
  ② 该端点忽略 `cache_control` 却按"缓存命中/未命中"两档计价 [S6][S2]，
     说明缓存确实会发生，但**是否体现在 Anthropic usage 字段里没有任何官方说明**。
- **落地建议**：Anthropic 面的计费解析必须**先实测抓一次真实 SSE 与 usage JSON**，
  并把原始 JSON 落进 `usage_events` 的旁路快照，再决定字段映射；不要照 Anthropic 规范硬编码。

## 6. 错误语义与限速

### 6.1 HTTP 状态码（官方错误码表）

| 状态码 | 描述 | 官方原因 | 官方解决方法 |
|---|---|---|---|
| 400 | 格式错误 | 请求体格式错误 | 按错误信息修改请求体 |
| 401 | 认证失败 | API key 错误 | 检查 API key / 先创建 key |
| 402 | **余额不足** | 账号余额不足 | 确认余额并充值 |
| 422 | 参数错误 | 请求体参数错误 | 按错误信息修改参数 |
| 429 | 请求速率达到上限 | 请求速率（TPM 或 RPM）达到上限 | 合理规划请求速率 |
| 500 | 服务器故障 | 服务器内部故障 | 等待后重试 |
| 503 | 服务器繁忙 | 服务器负载过高 | 稍后重试 |

[S3]

- **错误体 JSON 形状：官方未记载**。错误码页只给"错误码 | 描述 | 原因 | 解决方法"四栏，
  没有给响应体 schema 或示例。[S3]
  → 网关要"上游错误码原样透传"（ADR-0047 决策 5）没有问题，但**错误体字段需实测抓一次**；
  文档也未说明 402/429 是否附带余额或重试时间信息。
- Responses 面另有**响应内的失败语义**：`status: failed` + `error {code, message}`，
  以及 `incomplete_details.reason`（`max_output_tokens` / `content_filter`）。[S8]
  → 上游 200 但语义失败的情况必须靠这两个字段判定，**不能只看 HTTP 状态码**。
- `tool_choice=required`/指定工具 在 Chat Completions 思考模式下 **400**。[S5]

### 6.2 限速与并发（官方）

- **并发限制按模型、按账号粒度计，与 API Key 无关**：

| 模型 | 并发限制 |
|---|---|
| `deepseek-flash` | 2500 |
| `deepseek-v4-pro` | 500 |

[S2][S4]

- "一个请求从发出后，到模型响应完成之前记为一个并发"；超出并发限制收到 **HTTP 429**；可提工单免费扩容。[S4]
- **`user_id` 隔离**（对我们做多用户网关很有用）：
  - 取值须匹配 `[a-zA-Z0-9\-_]+`，**最大长度 512**，不得含隐私信息。[S4]
  - 三个用途：内容安全隔离、**KVCache 隔离**、调度隔离。[S4]
  - 传参位置：OpenAI Chat Completions 放在请求体顶层 `user_id`（用 OpenAI SDK 时需塞 `extra_body`）；
    **Anthropic 面放在 `metadata.user_id`**；Responses 面用顶层 `user` 字段。[S4][S5][S8]
  - 配额已扩容的账号，会按每个 `user_id` 单独限并发（deepseek-flash 2500 / v4-pro 500）。[S4]
- **保活与超时**：非流式请求会持续返回空行；流式请求持续返回 SSE 注释 `: keep-alive`；
  **10 分钟后仍未开始推理，服务端关闭连接**。自行解析 HTTP 响应时要能容忍这些空行/注释。[S4]
  → 对我们：网关的读超时与 SSE 解析必须容忍 keep-alive 注释与空行。
- **TPM/RPM 的具体数值：官方未给出**（错误码页提到 TPM/RPM，但只有并发限制有具体数字）。[S3][S4]

## 7. 计价（官方价目表，单位：元 / 百万 tokens）

| 项 | deepseek-flash 空闲 | deepseek-flash 高峰 | deepseek-v4-pro 空闲 | deepseek-v4-pro 高峰 |
|---|---|---|---|---|
| 输入（缓存命中） | 0.02 | 0.04 | 0.15 | 0.30 |
| 输入（缓存未命中） | 1 | 2 | 4.5 | 9.0 |
| 输出 | 4 | 8 | 13.5 | 27.0 |
| 并发限制 | 2500 | | 500 | |

[S2]

- **错峰折扣**：空闲时段价格为高峰时段价格的**一半**。**高峰时段 = 北京时间周一至周五
  （不含中国法定节假日）9:00–12:00、14:00–18:00**；其余时段（含周末与法定节假日全天）为**空闲时段**。[S2]
- **缓存价差**：以 flash 为例，命中 0.02~0.04 元/百万 vs 未命中 1~2 元/百万，
  相差约 **50 倍**——缓存命中率直接决定毛利。[S2][S16]
- 上下文缓存（磁盘缓存）**对所有用户默认开启**，无需改代码；缓存"尽力而为"，不保证 100% 命中；
  缓存构建秒级，闲置几小时到几天后自动清空。[S16]
- **扣费规则**：扣减费用 = token 消耗量 × 模型单价；从充值余额或赠送余额扣减，
  **两者同时存在时优先扣赠送余额**。[S2]
- 计量口径（官方）："我们将根据模型输入和输出的总 token 数进行计量计费。"[S2]
- `claude-opus*` 映射到 `deepseek-v4-pro` 时**按 V4 Pro 价计费**。[S6][S7]
- 模型版本与容量：`deepseek-flash` = DeepSeek-V4.1-Flash，上下文 1M、最大输出 384K；
  `deepseek-v4-pro` = DeepSeek-V4-Pro-0813，同样 1M / 384K。[S2][S13]
- 官方保留调价权，建议价格表配置化（现 ADR-0047 决策 6 已如此）。[S2]

## 8. 与仓库现有假设逐条核对

| # | 仓库假设 | 官方文档结论 | 判定 |
|---|---|---|---|
| 1 | 主模型名 `deepseek-flash`（旧名 `deepseek-v4-flash` 已退役），另有 `deepseek-v4-pro` | 模型名请用 `deepseek-flash`；**旧名 `deepseek-v4-flash`、`deepseek-v4-flash-vision-exp` 仍可调用**，但对应模型已下线，请求由 V4.1-Flash 提供服务、按 Flash 价计费；另一个模型官方名为 `deepseek-v4-pro` | ✅ 支持，措辞需修正：旧名是**仍接受并被路由到 V4.1-Flash**，不是报错/不可用 [S1][S2][S12][S19] |
| 2 | Responses API 自 2026-08 起由官方支持，端点 `POST https://api.deepseek.com/responses` | 官方更新日志：**2026-07-31** V4-Flash 正式版"原生支持 Responses API 格式并针对性适配 Codex"；**2026-08-13** V4-Pro 更新条目同样写"原生支持 Responses API"。参考页确为 `POST /responses` | ✅ 端点与能力成立；时间点更准确为 **2026-07-31（Flash）/ 2026-08-13（Pro）**，且 2026-08 有官方公告可引 [S12][S8][S11] |
| 3 | Responses API 无状态：不支持 `previous_response_id` / `conversation` / `store` | 参考页原话 "The API is stateless: responses and conversations are not stored on the server"；兼容表逐条：`previous_response_id` 不支持、`conversation` 不支持、`store` 不支持（响应恒 `store: false`） | ✅ **完全一致**，且有逐字一手来源 [S8][S11] |
| 4 | 流式结束事件是 `response.completed` / `response.incomplete` / `response.failed`，没有 `data: [DONE]` | 事件表把这三个列为"最后一个事件"；指南两处原文："流以 response.completed / response.incomplete / response.failed 事件结束，没有 data: [DONE] 消息" | ✅ **完全一致**。补充：**Chat Completions 官方明确以 `data: [DONE]` 结尾**（Anthropic 端点未记载），网关必须按端点分流终态标记 [S10][S11][S5] |
| 5 | thinking 默认开启，`reasoning.effort` 可调；thinking 下 `temperature`/`top_p` 无效 | 思考模式默认打开、effort 默认 `high`；三套格式的控制参数：Chat = `thinking.type` + `reasoning_effort`，Responses = `reasoning.effort`，Anthropic = `reasoning.effort` + `output_config.effort`。思考模式不支持 `temperature`（传入不报错但不生效）；**`top_p` 恰好相反：只在思考模式下生效（下限 0.95），非思考模式恒 1.0** | ⚠️ 前半 ✅；**`top_p` 的后半需修正**——不是"thinking 下无效"，而是 **thinking 下 `top_p` 才有效（0.95–1.0），非 thinking 下才被忽略** [S15][S5][S8] |
| 6 | function 名限制 `^[a-zA-Z0-9_-]+$` | Responses API 原文：非空、≤128 字符、匹配 `^[a-zA-Z0-9_-]+$`、全局唯一。Chat Completions 措辞为"必须由 a-z、A-Z、0-9 字符组成，或包含下划线和连字符，最大长度 128 字符"（未给正则，未要求全局唯一） | ✅ Responses 面逐字一致；Chat 面是等价描述但**更宽松**（无正则、无唯一性要求）[S8][S5] |
| 7 | 图片输入支持 `input_image`（base64 data URL 或 http(s) URL），仅限 user/developer 消息与 function_call_output | 原文：`image_url` 为 http(s) URL（≤8192 字符）或 base64 data URL（`data:image/jpeg;base64,...`），JPEG/PNG/GIF/WebP；可出现在 user/developer 消息与 `function_call_output` / `custom_tool_call_output` 的 `output` 中；**system / assistant 消息中的图片返回 400**；文件输入不支持 | ✅ **完全一致**。补充：① 仅 `deepseek-flash` 支持（`deepseek-v4-pro` 纯文本）；② 还有 `file_id`（Files API）第三种方式 [S10][S8][S13] |
| 8 | （ADR-0047 隐含）OpenAI 面 base_url 可带 `/v1` 由网关兼容 | 官方文档一律写 `https://api.deepseek.com`，正文**未出现 `/v1`** | ⚠️ 官方未记载 `/v1` 形态；用户实测 `/chat/completions` 返回 200，但 `/v1` 前缀**没有一手依据**，需实测 [S1][S2] |
| 9 | （ADR-0047 决策 5）中转只解析 `response.completed` 的 usage | 官方：`response.incomplete` / `response.failed` 也携带完整 response 对象（含 usage） | ⚠️ 建议把另两个终态事件也纳入计费旁路，否则截断/失败的请求 usage 会丢 [S10][S8] |

## 9. 专项回答：本机实测的这些能力，官方文档是否已记载

| 本机实测观察 | 官方文档是否记载 | 出处与原文要点 |
|---|---|---|
| `POST https://api.deepseek.com/responses` 返回 200 | **已记载** | 参考页标题即 `POST /responses`；指南给出 `client.responses.create(...)` 可运行示例 [S8][S10][S11] |
| `POST https://api.deepseek.com/chat/completions` 返回 200 | **已记载** | 首页 curl 示例完整写出该 URL；参考页标题 `POST /chat/completions` [S1][S5] |
| Responses usage 形状（`input_tokens` / `input_tokens_details.cached_tokens` / `output_tokens` / `output_tokens_details.reasoning_tokens` / `total_tokens`） | **已记载** | 参考页 usage schema 与示例；指南"Token 用量在 usage 中返回"两条 [S8][S10] |
| Chat Completions usage 形状（`prompt_tokens` / `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` / `completion_tokens` / `*_details`） | **已记载** | 参考页 usage schema 与流式示例最后一个块；另有《上下文硬盘缓存》专段 [S5][S16] |
| 「Responses API 自 2026-08 起官方支持」 | **已记载**（时间点更早） | 更新日志 2026-07-31（V4-Flash）与 2026-08-13（V4-Pro）两条 [S12] |

**结论**：上表这 5 项本机实测能力**全部能在官方文档中找到对应记载**，没有"文档缺失但接口可用"的情况。
真正属于文档缺口的只有三处，且都不在这 5 项之内：
① Anthropic 端点的响应 / SSE / usage 形状（§2.4、§5.3）；
② 各端点错误体的 JSON 形状（§6.1）；
③ 官方文档未记载 `base_url` 带 `/v1` 的形态（§3）。

## 10. 核对方法与局限

- **一手来源**：全部取自 `api-docs.deepseek.com`（DeepSeek 官方文档站）与官方更新日志；
  引用编号见 §11。Anthropic 官方 API 手册（S20）仅作为"格式定义"的参照，非 DeepSeek 一手来源，
  凡涉及 DeepSeek 端行为的结论都以 DeepSeek 文档为准。
- **关于本机实测的定位**：用户此前在本机对 `POST /responses` 与 `POST /chat/completions` 测得 200，
  且 usage 形状与 §5.1 / §5.2 一致。这与官方文档**不矛盾**——官方文档确实已记载这两个端点及其
  usage 形状（[S8][S5]），本机实测只是对文档的印证；**未见于官方文档的是 Anthropic 端点的
  响应形状与 usage（§5.3）以及错误体形状（§6.1）**，这两项仍属文档缺口。
- **抓取方式**：本机 `api-docs.deepseek.com` 解析到代理地址 `198.18.0.168`（非公网 IP），
  `web_fetch` 会拒绝；因此正文引用的原文用 PowerShell `Invoke-WebRequest` 直连抓取 HTML
  并抽取 `<article>` 正文后逐行核对（含中英对照的同一页面）。
- **本报告未验证、需要实测的项**（明确列出，不要当成结论）：
  1. `/anthropic` 端点的**完整路径**（`/v1/messages`）、**SSE 事件名清单**与**流终止标记**；
  2. **Anthropic 端点的 usage 字段形状**（是否含 `cache_read_input_tokens` /
     `cache_creation_input_tokens`，还是别的形状）；
  3. **各端点错误体的 JSON 形状**（400/401/402/422/429/500/503 的 body 字段）；
  4. `base_url` 带 `/v1` 能否工作；
  5. **Responses 面在 thinking 下 `tool_choice=required` / 指定工具**是否与 Chat 面一样 400
     （两处官方文档口径不一致）；
  6. 流式下 `input_tokens_details` / `output_tokens_details` 是否恒存在；
  7. `deepseek-flash[1m]` 这个模型名写法的语义（官方仅在 Claude Code env 示例里出现，未解释）。
- 以上 7 项建议在 M3 真实链路里各抓一次原始报文固化进测试（仓库已有
  `cargo test --test live_api -- --ignored` 的 live 复验通道可复用）。

## 11. 来源清单

| 编号 | 标题 | URL |
|---|---|---|
| S1 | 首次调用 API（Your First API Call） | https://api-docs.deepseek.com/zh-cn/ |
| S2 | 模型 & 价格（Models & Pricing） | https://api-docs.deepseek.com/zh-cn/quick_start/pricing/ |
| S3 | 错误码（Error Codes） | https://api-docs.deepseek.com/zh-cn/quick_start/error_codes/ |
| S4 | 限速与隔离（Rate Limit & Isolation） | https://api-docs.deepseek.com/zh-cn/quick_start/rate_limit/ |
| S5 | Chat Completions API 参考 | https://api-docs.deepseek.com/zh-cn/api/create-chat-completion/ |
| S6 | 使用 Anthropic API | https://api-docs.deepseek.com/zh-cn/guides/anthropic_api/ |
| S7 | 接入 Claude Code | https://api-docs.deepseek.com/zh-cn/quick_start/agent_integrations/claude_code/ |
| S8 | Responses API 参考（POST /responses） | https://api-docs.deepseek.com/api/create-response/ |
| S9 | 接入 Codex | https://api-docs.deepseek.com/zh-cn/quick_start/agent_integrations/codex/ |
| S10 | 使用 Responses API 指南（中文版，含完整 SSE 事件表与参数兼容表） | https://api-docs.deepseek.com/zh-cn/guides/responses_api/ |
| S11 | 使用 Responses API 指南（英文版，同样含兼容表） | https://api-docs.deepseek.com/guides/responses_api/ |
| S12 | 更新日志（Change Log：2026-07-31 / 08-13 / 08-21 / 09-10） | https://api-docs.deepseek.com/zh-cn/updates/ |
| S13 | 获取模型列表（GET /models） | https://api-docs.deepseek.com/zh-cn/api/list-models/ |
| S14 | 图像理解（Vision，含图片限制表） | https://api-docs.deepseek.com/zh-cn/guides/vision/ |
| S15 | 思考模式（Thinking Mode，含 effort 映射与参数生效规则） | https://api-docs.deepseek.com/zh-cn/guides/thinking_mode/ |
| S16 | 上下文硬盘缓存（Context Caching，含 `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens`） | https://api-docs.deepseek.com/zh-cn/guides/kv_cache/ |
| S17 | 多轮对话（Multi-round Conversation，说明每轮需回传全量历史） | https://api-docs.deepseek.com/zh-cn/guides/multi_round_chat/ |
| S18 | Token 与用量计算 | https://api-docs.deepseek.com/zh-cn/quick_start/token_usage/ |
| S19 | News：DeepSeek-V4.1-Flash 发布（2026-09-10） | https://api-docs.deepseek.com/zh-cn/news/news260910/ |
| S20 | Anthropic 官方 API 手册（格式定义参照，非 DeepSeek 一手来源） | https://docs.anthropic.com/en/api/messages |

> 仓库内相关留痕：`docs/adr/0045-single-deepseek-model.md`、`docs/adr/0047-server-account-package-relay.md`、
> `docs/adr/0020-responses-api-main-model.md`。
>
> 一手来源的置信度说明：S1–S19 均为 DeepSeek 官方站点（文档站 + 更新日志 + 官方新闻页，
> 均为 `api-docs.deepseek.com` 域名）。**S20 未被正文引用**：它是 Anthropic 官方手册，
> **不是 DeepSeek 一手来源**，列在这里只为说明"Anthropic 原生格式长什么样"；
> 凡涉及 DeepSeek 侧行为的结论一律以 S1–S19 为准。

---

## 附录 A：本机实测（2026-09-29）

**为什么需要这一节**：正文已指出，Anthropic 端点的响应形状（usage 字段、SSE 事件名）官方文档**没有记载**，错误体形状与 `/v1` 前缀形态同样缺失。这三个缺口只能靠实测补齐，而它们恰好是网关计费解析的直接依据。

**方法与留痕**：用真实 key 对三个端点各发一次最小流式请求，原始字节原样固化为测试夹具：

| 夹具 | 来源端点 |
|---|---|
| `server/tests/fixtures/deepseek_responses_20260929.sse` | `POST https://api.deepseek.com/responses` |
| `server/tests/fixtures/deepseek_chat_completions_20260929.sse` | `POST https://api.deepseek.com/chat/completions` |
| `server/tests/fixtures/deepseek_anthropic_20260929.sse` | `POST https://api.deepseek.com/anthropic/v1/messages` |

夹具仅含模型对「1+1=?」的回答，已扫描确认无密钥残留。它们的用途是**充当 mock 上游**，让中转链路的集成测试不依赖真实 key 与网络（CI 里没有 key）。

### A.1 三个协议的 usage 落点（计费解析的唯一依据）

| 面 | 路径 | 终态标记 | usage 位置 | 缓存命中字段 |
|---|---|---|---|---|
| Responses | `POST /responses` | `response.completed` / `response.incomplete` / `response.failed`（**无** `data: [DONE]`） | `data.response.usage` | `input_tokens_details.cached_tokens` |
| Chat Completions | `POST /chat/completions` | `data: [DONE]`；带 usage 的块在 `finish_reason` 那块之后 | `data.usage` | `prompt_cache_hit_tokens` / `prompt_cache_miss_tokens` |
| Anthropic | `POST /anthropic/v1/messages` | `message_delta`（随后 `message_stop`） | **分两处**：输入在 `message_start.message.usage`，输出终值在 `message_delta.usage` | `cache_read_input_tokens` / `cache_creation_input_tokens` |

实测原始值（同为「1+1=?」请求）：

```jsonc
// Responses: response.completed
"usage": {"input_tokens":12, "input_tokens_details":{"cached_tokens":0},
          "output_tokens":1, "output_tokens_details":{"reasoning_tokens":0}, "total_tokens":13}

// Chat Completions: 最后一个 data 块
"usage": {"prompt_tokens":38, "completion_tokens":36, "total_tokens":74,
          "prompt_tokens_details":{"cached_tokens":0},
          "completion_tokens_details":{"reasoning_tokens":34},
          "prompt_cache_hit_tokens":0, "prompt_cache_miss_tokens":38}

// Anthropic: message_start.message.usage（输入，output_tokens 此时恒为 0）
{"input_tokens":38, "cache_creation_input_tokens":0, "cache_read_input_tokens":0,
 "output_tokens":0, "service_tier":"standard"}

// Anthropic: message_delta.usage（终值）
{"input_tokens":38, "cache_creation_input_tokens":0, "cache_read_input_tokens":0,
 "output_tokens":30, "service_tier":"standard"}
```

### A.2 运行期细节（文档未记载，但直接影响透传实现）

- **Anthropic 面会下发 `ping` 事件**（本次样本 1 个）。SSE 解析器必须容忍未知事件类型——见到没见过的 `event:` 不能断流或报错。
- **Chat Completions 不传 `stream_options.include_usage` 也返回非空 usage**（连测两次均有）→ 网关**不必改写**客户端请求体去索要 usage，省掉一处请求改写与一类兼容风险。
- **流式保活**：官方《限速与隔离》说明流式请求会持续下发 `: keep-alive` 注释与空行。解析器必须跳过注释行与空行，且这些痕迹不能被当成数据或终态。
- **`input_tokens` 语义在两个面上不同**：Responses 的 `input_tokens` **已包含**缓存命中（客户端注释：未命中 = 输入 − 命中）；Anthropic 的 `input_tokens` **不含**缓存读取，总输入 = `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`。网关必须把三面**归一化**成同一口径，否则同一用户换个协议用，计费就会漂移。
- **Anthropic 模型映射的副作用**：传 `claude-opus*` 会被上游映射到 `deepseek-v4-pro`，而 `deepseek-v4-pro` **不支持图片输入**。带图请求若同时走 Anthropic 面 + opus 模型名，会撞上这个组合。
- **用户隔离字段三面不同名**：Responses 用顶层 `user`，Chat Completions 用 `user_id`，Anthropic 用 `metadata.user_id`（字符集 `[a-zA-Z0-9\-_]+`，≤512）。网关把平台用户 id 填进对应字段，即可拿到上游的 **KVCache 隔离、内容安全隔离与调度隔离**——这是免费拿到的多租户隔离能力，值得默认开启。

### A.3 对 S3（中转）的直接结论

1. **三个面全部可以透传**，差异只在「下游路径 → 上游路径」映射与「usage 提取器」两处 → 协议适配器是薄层，无需双向翻译。**这推翻了「Anthropic 必须双向翻译」的初判**，Anthropic 可以纳入首期。
2. 内部计费口径统一为四元组 `{input_total, cached, output, reasoning}`，由各适配器从各自字段填充并归一化（语义差见 A.2）。
3. 终态判定按面分流，且 Responses 面的终态是**集合**（completed / incomplete / failed），不是单一事件名。
