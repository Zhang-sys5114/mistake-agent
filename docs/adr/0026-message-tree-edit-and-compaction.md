# 消息树编辑/切分支与上下文压缩落地

消息树（ADR-0007）补齐两条 RPC：`edit_message{message_id,text}` 与 `switch_branch{message_id}`。

- `derive_branch`：仅允许编辑 assistant 消息；复制新 id、parent 不变、文本替换，编辑点之后的消息保留在 JSONL 但不再属于活跃路径（历史不截断）；`SessionMeta.active_path` 指向新分支末端；返回新活跃路径。
- `switch_branch`：沿 parent 链回溯到根，切换 `active_path`，返回该路径。
- `read_path` 遵循 active_path：有则返回根→末端链，无则退化为线性全链（兼容旧数据）。
- 审计新增 `MessageEdited`/`BranchSwitched`/`Compaction` 记录。

压缩：上下文用量按字符数/2 粗估 token，达到窗口 75%（默认 131072 token 窗口）且最近 15 条之外的旧消息存在时触发；旧消息由 Summarizer 生成摘要并以 system 消息写入会话，最近 15 条保留；摘要为空重试一次，仍失败下回合再试。压缩发生在回合结束，`TurnOutcome.compacted` 通知 RPC 层发 `Event::Compaction`、审计并投递 `CompactionDone` 中断。

落盘去重：RPC 只追加"本回合新增"的消息（按消息 id 与回合前集合差集），压缩摘要按新增消息落盘，旧消息不重复追加。
