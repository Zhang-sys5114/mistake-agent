# 守卫模型与摘要器真实接入

> 注（ADR-0044 修订）：`LlmGuard` 及 `guard_prompt()` 已退役——守卫模型的最后一个调用方（会话切换决策）已删除，`GuardModel` trait 整个移除。`LlmSummarizer` 保留并成为唯一摘要器，同时服务交接摘要（用户新建会话时按需）与上下文压缩。`guard_model` 只是本文末尾提到的占位计划、从未在 `settings.json` 落地，该计划随守卫模型一并作废。

SessionScheduler 从 StubGuard/StubSummarizer 升级为 LLM 生产实现（默认复用主模型，经 Responses API）。

- ~~`LlmGuard`：输入 goal/summary/new_text 序列化为 JSON，配合 `guard_prompt()` 与 json_schema 强制 `{action, goal}` 输出；reasoning effort 置 none。解析容忍 ```json 围栏。模型错误、超时、输出无法解析时由调度层降级为 `Continue`（存疑即继续，Q17），不中断用户回合。~~（ADR-0044 退役）
- `LlmSummarizer`：把消息压缩成 ≤300 字任务摘要，保留错题 id、知识点、未完成事项；输入截断到 12000 字符；模型失败/超时降级为 stub 式摘要（消息数 + 目标）。
- Kernel 组装时 scheduler 与 agent loop 共享同一个 InterruptBus（ADR-0023 消费链闭环）；摘要器直接持有主模型服务，不进入主模型上下文。两个调用方（交接摘要、上下文压缩）参数完全相同且摘要器无状态，故共享同一个 `Arc<dyn Summarizer>` 实例（ADR-0044 顺带收敛）。

摘要调用属于调度开销，默认复用主模型；未来可配独立小模型。
