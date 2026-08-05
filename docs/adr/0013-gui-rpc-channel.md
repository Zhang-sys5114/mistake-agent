# GUI 通信：stdio JSONL 结构化 RPC，trigger_command 唯一命令通道

> 更新（2026-08-05）：sidecar 已彻底移除，GUI 与 kernel 改为**进程内** mpsc + Tauri Channel 桥接（ADR-0029），协议帧格式（JSON Lines）与 trigger_command 唯一命令通道的结论不变。

Tauri 以 sidecar 进程拉起 kernel，双方通过 stdio 的 newline-delimited JSON 通信，不占用本地端口。GUI 请求包括 send_user_message、trigger_command、edit_message、switch_branch、abort、get_state；kernel 向 GUI 输出事件流（message_delta、tool_start/end、turn_end、session_switched、memory_changed、compaction、error）。命令的唯一通道是 trigger_command(entry, params)：GUI 不传递可任意执行的文本命令，kernel 校验 EntryPoint 与 CallerPolicy，前端门禁由此结构性成立。
