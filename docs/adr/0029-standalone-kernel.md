# 0029 — Standalone：kernel 内嵌 GUI 进程

日期：2026-08-04
状态：已采纳

> 更新（2026-08-05）：sidecar 已**彻底移除**——`src/bin/sidecar.rs` 与 Cargo 的 sidecar bin 条目删除，协议帧格式保持不变（前端零改动），mistake-agent 成为唯一二进制交付物。

## 背景

M3 起 GUI 通过拉起独立 `sidecar` 进程（stdio JSONL）与 kernel 通信，mistake-agent 运行强依赖同目录存在 sidecar 二进制。用户明确要求：**不允许该依赖，mistake-agent 必须 standalone（单二进制即可运行）**。

## 决策

1. **进程内 kernel**：`src/main.rs` 的 `start_kernel` 直接在 Tauri async runtime 上创建 `Kernel`（`Kernel::new`），不再 `Command::new(sidecar)`。
2. **桥接**：前端请求经 `kernel_send` 投递到 `mpsc::UnboundedChannel<String>`，请求循环解析 `RpcRequest` → `Kernel::handle`；响应帧与 `EventSink`（ChannelEventSink）的事件帧统一经 Tauri `Channel<String>` 推回前端。协议帧格式（JSON Lines）与 sidecar 时代完全一致，前端零改动。
3. **sidecar 降级为调试工具**：`src/bin/sidecar.rs` 与 bin 声明保留，仅用于 CLI 管道测试/脚本调试；GUI 不再引用。
4. **打包简化**：Windows 打包无需再捆绑 sidecar 可执行文件。

## 后果

- 单进程内存模型：回合任务仍由 tokio 调度；Kernel 内部并发语义不变（state 锁保证单回合）。
- 崩溃面收窄：不再有进程拉起/EOF/管道竞争问题（原 Bug #2/#7 场景消失）。
- sidecar CLI 仍可独立验证 kernel 链路（真实 API 测试不依赖 GUI）。
