# Mistake Agent Context

面向中学生的本地错题管理与辅助学习 Agent（v2）。采用 OS 式分层：内核负责核心调度，内核插件处理敏感资源与能力，用户插件提供业务功能。

## Language

**Kernel（内核）**:
本地 Agent 引擎的核心调度层，负责 Agent loop、会话生命周期、工具注册与调度、事件/RPC 和指令/技能加载；不实现任何业务能力。
代码布局：`src/kernel/agent/`（核心调度层）+ `src/kernel/plugin/`（内核插件，一插件一文件夹）。
_Avoid_: 核心、引擎（含义过宽）

**Agent core（Agent 核心）**:
可跨应用复用的通用 Agent 运行时：Agent loop、工具注册与调度、会话生命周期、模型运行时抽象与审计护栏；即 Kernel 剔除应用专属内核插件后余下的部分。以独立 crate `so-lite-agent` 提供（M1/M2 已落地：本仓库解耦 + `so-lite-agent/` 骨架；M3-M5 待办），新 Agent 通过 `cargo add so-lite-agent` 复用，内核插件由使用方自行编写。
_Avoid_: 引擎、Agent 内核（与 Kernel 混用）

**Kernel plugin（内核插件）**:
运行在内核信任边界内的特权子系统，负责敏感资源与能力，如会话存储和验算运行时。
经 `KernelPlugin` 两段式契约注册（info + register，ADR-0035），注册上下文为全量服务句柄。
_Avoid_: 内核级插件（口语）、系统服务

**KernelPlugin contract（内核插件两段式契约）**:
内核插件的注册机制，与用户插件 `UserPlugin` 同形：`info()` 声明 namespace、`provides`（提供的 ServiceId）与工具/命令/事件，`register(ctx)` 绑定 handler；与用户插件共用同一张注册表校验（namespace/wire 唯一、CallerPolicy、懒/急加载），注册上下文 `KernelContext` 注入全量服务句柄。
_Avoid_: 直连注册（ADR-0021 旧表述）

**Disabled plugin（禁用插件）**:
插件目录根部存在 `disabled` 标记文件、不参与构建与注册的插件；移除标记即恢复（ADR-0036）。
_Avoid_: 卸载、下架（暗示运行时卸载，编译期内置无此语义）

**User plugin（用户插件）**:
通过内核注册工具、命令与事件回调提供业务能力的插件，如批改、练习、复盘；其回调由 kernel 主动调用，但不直接接触敏感资源。
_Avoid_: 业务插件（过早限定业务范围）、用户态插件（口语）

**Service（服务）**:
内核插件向 kernel 提供的受控能力（v2 为会话存储、验算运行时、记忆、模型），由内核插件在 info 中以 `provides` 声明，用户插件只能通过服务句柄访问。
_Avoid_: API（含义过泛）

**Service handle（服务句柄）**:
kernel 按能力声明注入用户插件的受限接口，只暴露该插件需要的方法，不暴露底层资源；等价于 OS 的文件描述符。
_Avoid_: 全局单例、直接依赖

**Capability declaration（能力声明）**:
用户插件注册时声明的服务依赖清单；kernel 据此校验并注入句柄。
_Avoid_: 依赖注入（框架术语）

**Trust boundary（信任边界）**:
内核与内核插件所在的受信区域；用户插件只通过服务句柄与之交互。
_Avoid_: 沙箱（沙箱专指验算运行时）

**LoadPolicy（加载策略）**:
插件在 info 中声明的初始化时机：eager（读取即加载）或 lazy（首次使用才加载），默认 lazy。
_Avoid_: 积极性（口语）

**ToolDef（工具定义）**:
工具在 info 中声明的元数据条目：短名、描述、参数 schema；实际执行由 register 阶段绑定的 handler 完成。
_Avoid_: 工具（指调用实例）

**Tool catalog（工具目录）**:
前端可见工具元数据（标题/分组/图标/描述/参数 schema）的唯一事实源，经 `list_tools` RPC 由 kernel 下发；前端不得自行维护工具名 → 展示信息映射。
_Avoid_: 前端工具表、硬编码图标/标题

**EntryPoint（入口点）**:
用户插件向 kernel 登记的调用入口，共三类：Tool（LLM 调度）、Command（GUI/用户调度）、Event（kernel 生命周期调度）。
_Avoid_: 回调（只指其中一类）、接口

**ToolError（工具错误）**:
工具调用失败时返回给 LLM 的结构化错误，含 code、message、retryable；retryable 表示是否值得模型换参数重试。
_Avoid_: 异常（Rust panic 语义不同）

**Turn（回合）**:
kernel 一次完整的 agent 执行单元：从输入触发开始，到模型自然停止或护栏中止结束，期间可多次调用工具。
_Avoid_: 会话（Session 是整个使用生命周期）

**ModelRuntime（模型运行时）**:
kernel 对 LLM provider 的统一抽象，提供流式消息与工具调用；v2 实现 OpenAI 兼容端点与 Ollama。
_Avoid_: Provider（指具体厂商适配器）

**Session（会话）**:
一次对话流的过程记录，JSONL 追加式持久化；平台层由来源路由派生 SessionKey，任务层由 LLM 通过内核 session 工具切换，对用户完全隐藏。
_Avoid_: 对话（用户视角的聊天）、聊天记录

**SessionKey（会话键）**:
标识一个会话的内部路由键；平台层由消息来源派生（v2 即本地 App 本身，未来聊天渠道按渠道与对端隔离），任务层切换时生成新键，不暴露给用户。
_Avoid_: 会话 ID（暗示用户可见）

**Session handoff（会话交接）**:
任务层切换会话时，kernel 在当前消息节点下分叉出「会话子树」，以「上一会话梗概」摘要节点开头（摘要整条链）的机制；摘要节点是模型上下文边界，旧会话内容不进新会话上下文。
_Avoid_: 迁移、续传

**Interrupt（内部中断）**:
内核组件（会话调度、settings、memory、compaction）向 agent loop 发出的环境变更信号，通知其下回合上下文需按新环境重组；回合边界消费，不抢占当前回合，真正需要立即打断的场景走取消链。
_Avoid_: 事件（Event 指面向 GUI 的播报）

**Wire name（模型可见名）**:
内部规范名 namespace::tool 经 :: → __（双下划线）映射后发给模型的工具名（如 grading::upload → grading__upload），函数名受 OpenAI 系 API 的 ^[a-zA-Z0-9_-]+$ 约束；内部名、审计名与 trigger_command 不变。
_Avoid_: 全名（指内部 namespace::tool）

**Session scheduler（会话调度）**:
独立的内核级模块（非服务插件），负责会话生命周期与任务层切换；**切换 = 树内分叉**（不新建 SessionKey）：start_new / 空闲超时 / session::switch 工具都在当前消息节点下挂一棵以摘要节点开头的会话子树（ADR-0030/0032）；持久化委托 storage 服务。
_Avoid_: 会话管理（易与用户可见的管理界面混淆）

**Guard model（守卫模型）**:
（已退役，ADR-0030）原设计由 Session scheduler 调用的独立调度模型；现切换决策全部归主模型：新消息到达先判断是否切换上下文（ADR-0032，先判断后回答），回合结束由 LlmTurnDecider 判断 continue / update_goal / start_new（失败降级 continue），回合内由主模型调用 session::switch 工具主动切换。
_Avoid_: 调度模型（易与主模型混淆）

**Goal（会话目标）**:
当前会话要完成的学习目标，由主模型在 start_new 时生成并写入会话元数据；主模型据此在 continue / update_goal / start_new 三动作间决策，存疑即继续。
_Avoid_: 任务名（过窄，Goal 可含更丰富描述）

**History route（历史路由）**:
模型浏览完整消息树的通道（列出会话 + 读取指定会话的消息树），与记忆路由并列；新会话上下文只含本会话子树（从摘要节点起算），完整旧记录按需翻阅。
_Avoid_: 聊天记录查询（口语）

**Message tree（消息树）**:
会话内消息的组织结构：每条消息有 id 与 parentId，JSONL 追加式存储；编辑消息或"重新生成"回答时在该点派生新分支，历史永不截断。
_Avoid_: 版本历史、对话树（口语）

**Active path（活跃路径）**:
消息树中从根到当前节点的唯一路径；LLM 上下文只包含活跃路径上的消息，旁支不进入上下文。
_Avoid_: 当前分支（口语）

**Memory route（记忆路由）**:
记忆按层级路径组织（学科/知识点/条目），模型通过 memory::save/show/remove 自行浏览与读写；数据根目录 memory/ 文件持久化（重启不丢）；上下文不注入记忆内容，只保留一行入口提示。
_Avoid_: 记忆检索（暗示向量/全文检索）、长期记忆（过于宽泛）

**Memory entry（记忆条目）**:
记忆目录中的一个具体条目，由路径定位、文本内容承载；写路径由 LLM 决定，路径由 memory 内核插件校验。
_Avoid_: 记忆文件（暗示实现细节）

**Command channel（命令通道）**:
GUI 触发已注册 EntryPoint 的唯一通道（trigger_command）；协议层不存在"执行任意命令"的接口，前端门禁由此结构性成立。
_Avoid_: 命令执行（暗示任意执行）

**ModelHandle（模型句柄）**:
kernel 注入用户插件的受限模型服务句柄，仅暴露带超时、abort 与审计的 complete 调用；凭据与 provider 适配永远不离开 kernel。
_Avoid_: 模型客户端、直接调 provider

**Settings（配置）**:
数据根目录 settings.json 的内容，由用户通过 App 设置界面独占写入，kernel 启动时读取；模型与插件没有任何配置访问通道。
_Avoid_: 配置文件（实现细节）、系统设置

**Compute backend（验算执行端）**:
compute 服务的实际执行位置（v2 为 GUI WebView 内的 Pyodide，经 Event::ComputeRequest / Method::ComputeResult 桥接，kernel 侧 BridgeCompute 等待回执并做超时/取消/审计）；Pyodide 即 WASM 沙箱。
_Avoid_: 沙箱（专指隔离形态）

**Audit（审计）**:
所有操作的强制记录（默认全覆盖），由 kernel 经 storage 服务写入数据根目录 audit/ 的追加式 JSONL；流式中间态不记，大内容以引用关联。
_Avoid_: 日志（过泛，包含调试日志）

**Diagnostic log（诊断日志）**:
分级诊断记录（DEBUG/INFO/WARN/ERROR/CRITICAL/PANIC），与审计分离，写入数据根目录 logs/；敏感值脱敏。
_Avoid_: 日志（与 Audit 混用）、审计日志

**Main model（主模型）**:
负责 agent loop 调度与对话的模型（v2 为 deepseek-v4-flash，经 DeepSeek Responses API 接入），在 settings 中配置 API_URL 与 API_KEY。
_Avoid_: 聊天模型（口语）

**Vision model（视觉模型）**:
负责图片理解与 OCR 的模型（v2 为硅基流动 SiliconFlow 的 qwen3-VL，经 Chat Completions 接入；Responses API 不支持图片输入），与主模型分开配置，经 ModelHandle 按用途调用。
_Avoid_: OCR 模型（只覆盖一部分用途）

**CallerPolicy（调用方策略）**:
EntryPoint 的调用方边界：UserAndModel（模型可调，用户必可调）或 UserOnly（仅用户可调，模型工具列表不可见且调度拒绝）。
_Avoid_: 权限（含义过泛）

**Data root（数据根目录）**:
本 Agent 所有数据与配置的统一存放目录（~/Documents/.mistake-agent）；不存在"项目"概念。
_Avoid_: 项目目录、工作区


**Learning task（学习任务）**:
学生在一次会话中要完成的学习单元（批改一次作业、一轮复习等），与会话一一对应。
_Avoid_: 任务（Task，易与工具任务混淆）

**Chemistry rendering（化学渲染）**:
前端 Markdown 中化学内容的渲染方式：KaTeX + 官方 mhchem 扩展（`\ce{}` / `\pu{}`）支持化学式、方程式、同位素与单位；结构式（键线式）由模型以 SMILES 代码块（```smiles）输出，前端 smiles-drawer 绘制 SVG；不支持 chemfig/TikZ 类结构式宏包（KaTeX 无 TikZ 引擎，需完整 LaTeX 才可编译）。
_Avoid_: 直接把 chemfig 当 KaTeX 宏包引入（会静默渲染失败）、让模型输出结构式图片或 Unicode 伪图形

**Mistake management state（错题管理状态）**:
错题本记录的轻量管理字段：`is_correct` 表示已掌握（复用原有字段），`pinned` 表示置顶，`deleted_at` 非空表示软删除；`grading::list` 默认隐藏已删除记录，`grading::remove` / `grading::remove_many` 只写 `deleted_at`，不物理删除。_Avoid_: 硬删除错题、为已掌握另建 `mastered` 字段
