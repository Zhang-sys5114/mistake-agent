# AI 对话前端：消息编辑 / 重新生成 / 版本切换实现调研

> 研究时间：2026-08-08。目标：搞清楚主流 AI 聊天前端（ChatGPT / DeepSeek 风格）的
> 「编辑用户消息就地替换并重新回答」「重新生成」「< / > 切换版本」是怎么实现的，
> 以及对我们（Vue 3 + Tauri + Rust kernel 消息树 + active_path）的落地建议。

## 1. 结论先行

- **主流开源实现（NextChat、LobeChat、Vercel AI SDK 官方示例）都是「平铺消息列表 +
  删除重发」模型**：重新生成 = 删掉旧回答（或旧问答对）再跑一次，**没有版本历史**。
- **真正有「版本切换」的产品（ChatGPT、DeepSeek、Claude）都是树/分支模型**：每次
  编辑或重新生成都保留旧版本（sibling 节点），界面只显示一条「活跃链」，用 < / >
  或版本号在兄弟版本间切换。这与本项目 Rust kernel 已有的消息树
  （id/parent_id + JSONL 追加 + `SessionMeta.active_path`）**结构一致**——模型没选错。
- 之前踩的坑（发新消息"消失"、箭头切不回来）不是数据模型问题，而是**前端链式渲染的
  事件时序**（turn_end 在落盘前发出）和**版本列表漏掉当前消息**导致的实现缺陷。

## 2. 一手来源与证据

### 2.1 Vercel AI SDK（官方文档）

- 文档：<https://sdk.vercel.ai/docs/ai-sdk-ui/chatbot>
- `useChat` 返回 `messages`（**平铺数组**）、`sendMessage`、`regenerate`。
- 官方对 `regenerate` 的定义：*"request the AI provider to reprocess the last message"*
  —— 即"重跑最后一条"，UI 上通常就是失败重试按钮；SDK 本身不提供版本历史，
  编辑/版本切换属于应用层自己实现（可用 `setMessages` 改数组后重发）。

### 2.2 NextChat / ChatGPT-Next-Web（开源，平铺替换派）

- 仓库：<https://github.com/ChatGPTNextWeb/ChatGPT-Next-Web>，commit `706a18b`
- 数据模型：`session.messages: ChatMessage[]`（flat list，只有 id/role/content）。
- 重新生成（`app/components/chat.tsx` 的 `onResend`）：
  - 目标是 assistant 消息 → 向上找最近一条 user 消息；
  - 目标是 user 消息 → 向下找最近一条 assistant 消息；
  - **把这对消息从数组里删掉，再重发 user 输入**（注释原文：
    *"3. delete original user input and bot's message / 4. resend the user's input"*）。
  - 结论：删除 + 追加，无版本历史。
- 编辑（同文件 `EditMessageModal` / 消息上的编辑按钮）：
  `chatStore.updateTargetSession(... m.content = newContent ...)` —— **原地改内容**，
  保留图片（multimodal content 重组），不生成新版本。

### 2.3 LobeChat（开源，平铺替换派 + 乐观更新）

- 仓库：<https://github.com/lobehub/lobe-chat>，commit `1404dd9`
- 数据模型：每个 topic 下平铺消息（`src/store/chat/slices/message/reducer.ts`）：
  `createMessage / updateMessage / deleteMessage / updateMessages`，无 parent 树、无版本。
- 重新生成：`src/store/chat/slices/agentRun/actions/entries/conversationControl.ts`
  的 `regenerateUserMessage`，配合「operation 系统」做**乐观更新**（先改 UI，再跑 agent）。
- 编辑：走 `updateMessage` 原地改 content。

### 2.4 ChatGPT / DeepSeek / Claude（闭源，树 + 活跃路径，观察行为）

- 编辑用户消息：就地替换显示并自动重新回答；旧版本仍可找回（< / > 或版本号切换）。
- 重新生成：新回答替换展示，可通过版本切换器翻看旧回答。
- 一次只显示一条链；切换版本后继续发送，从**所选版本**继续（服务端 active path 跟着走）。
- 这正是消息树 + active_path 的语义；闭源无源码，以上为产品行为观察。

### 2.5 DeepSeek 网页版前端（闭源，打包产物逆向）

> 来源：<https://chat.deepseek.com/> 构建产物（commit `a610cb2`）：
> `https://fe-static.deepseek.com/chat/static/main.7863ea53ee.js`
> 以下结论均来自对打包 JS 的静态分析。

**数据模型 = 消息树 + 逐节点版本指针**：

- 客户端消息字段：`{ id: message_id, parentId: parent_id, role, status, childIds,
  currentChildIndex, isEditing, banRegenerate, accumulatedTokenUsage, ... }`。
- 会话结构：`{ id, title, messageStore: Map<id, message>, rootBranchIds,
  rootBranchIndex, ... }`。根层有多个版本（`rootBranchIds`），每个节点记录
  `childIds`（子版本列表）+ `currentChildIndex`（当前选中的子版本下标）。
- 活跃链 = `getMessagePath`：从 `rootBranchIds[rootBranchIndex]` 出发，逐层取
  `childIds[currentChildIndex]` 直到叶子。

**版本切换（navigateMessageBranch(sessionId, messageId, dir)）**：

- 目标在根层：`rootBranchIndex = mod(rootBranchIds, rootBranchIndex + dir)`；
- 目标在非根：`parent.currentChildIndex = mod(parent.childIds, parent.currentChildIndex + dir)`；
- 即 **+1/-1 对版本列表取模循环**，与我们的 < / > 设计一致。

**分支激活**：给出一条消息 id，沿 parent 链向上逐层把祖先的
`currentChildIndex` / `rootBranchIndex` 指向该子节点（点某条旧消息 = 激活它的分支）。

**编辑 / 重新生成 / 续写都是同一条 SSE 流**：

- API：`/api/v0/chat/edit_message`、`/api/v0/chat/regenerate`、
  `/api/v0/chat/continue`、`/api/v0/chat/completion`、`/api/v0/chat/resume_stream`、
  `/api/v0/chat/history_messages`（拉全量树同步）。
- 客户端控制器动作：`startCompletion / regenerateMessage / editMessage /
  continueCompletion / resendMessage / feedbackMessage / resumeMessage / abortCompletion`。
- 编辑 = `scene:"editMessage"` 走同一 completion 管线（含 PoW 质询头、附件准备、
  SSE delta 流），完成后服务端返回新的消息树。
- `banRegenerate` 字段控制单条消息是否禁止重新生成；续写仅允许叶子
  （错误码 `CURRENT_MESSAGE_IS_NOT_A_LEAF_MESSAGE`）。

**与我们实现的对应关系**：

- DeepSeek 的 `childIds/currentChildIndex/rootBranchIndex` ≈ 我们的
  `Message.parent_id` + `SessionMeta.active_path`，区别是**指针存在每个节点上**，
  切换任意一层只改那一层的指针，天然支持任意深度版本浏览；
  我们只存一个"链末端"，切版本时要自己补"最深后代"才能显示完整旧分支。
- 后续若重做，可参考把"当前分支指针"从单一 active_path 改为按父节点存
  `child_ids + current_child_index`（前端状态即可，不必进 kernel）。

## 3. 对我们（Vue + Tauri + Rust 消息树）的落地建议

1. **保留树 + active_path，前端只渲染一条活跃链**（我们上一轮的链式渲染方向是对的）。
2. **版本列表必须包含"当前消息"**：每个气泡带上 `versions`（同 parent 的所有消息
   含自己，按 created_at 排序）；< / > 按当前版本在列表中的下标前后循环，
   才能"切出去再切回来"。只给 siblingIds（不含自己）必然切不回来。
3. **切换是前端本地查看**（不动服务端 active_path），符合"浏览旧版本"心智；
   发新消息/编辑时回到服务端活跃路径（DeepSeek 若要从旧版本继续，则另走
   `switch_branch` RPC 持久化）。
4. **事件时序**：`turn_end` 必须在消息落盘 + active_path 推进之后发出（我们已修过，
   撤回分支时一并丢了，重做时要带上），否则前端回读时新消息不在链上 → 消失。
5. **编辑时不要整屏替换**：edit 提交后立即在本地把被编辑消息换成新版（保留其余链），
   再流式接入新回答；避免"只剩一条消息"的塌缩感。
6. 小技巧：版本切换目标 = 目标兄弟**分支最深后代**（沿 created_at 最新子消息下行），
   保证切到旧版本时整个旧分支完整显示（问题+回答都在）。

## 4. 参考

- Vercel AI SDK Chatbot 指南：<https://sdk.vercel.ai/docs/ai-sdk-ui/chatbot>
- NextChat：<https://github.com/ChatGPTNextWeb/ChatGPT-Next-Web>
  `app/components/chat.tsx`（onResend / EditMessageModal）
- LobeChat：<https://github.com/lobehub/lobe-chat>
  `src/store/chat/slices/message/reducer.ts`、
  `src/store/chat/slices/agentRun/actions/entries/conversationControl.ts`
- DeepSeek 网页版：<https://chat.deepseek.com/> 打包产物
  `main.7863ea53ee.js`（commit `a610cb2`，`navigateMessageBranch` /
  `getMessagePath` / `getMessageBranchCount` / `getMessageBranchIndex`）
- 本项目现有机制：PROJECT.md §5 消息树、ADR-0007/0026（追加式 JSONL、编辑派生分支、
  active_path、switch_branch）
