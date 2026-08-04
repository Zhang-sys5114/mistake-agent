# 守卫模型与摘要器真实接入

SessionScheduler 从 StubGuard/StubSummarizer 升级为 LLM 生产实现（默认复用主模型，经 Responses API）。

- `LlmGuard`：输入 goal/summary/new_text 序列化为 JSON，配合 `guard_prompt()` 与 json_schema 强制 `{action, goal}` 输出；reasoning effort 置 none。解析容忍 ```json 围栏。模型错误、超时、输出无法解析时由调度层降级为 `Continue`（存疑即继续，Q17），不中断用户回合。
- `LlmSummarizer`：把消息压缩成 ≤300 字任务摘要，保留错题 id、知识点、未完成事项；输入截断到 12000 字符；模型失败/超时降级为 stub 式摘要（消息数 + 目标）。
- Kernel 组装时 scheduler 与 agent loop 共享同一个 InterruptBus（ADR-0023 消费链闭环）；守卫与摘要器直接持有主模型服务，不进入主模型上下文。

守卫/摘要调用属于调度开销，默认复用主模型；未来可配独立小模型（guard_model 配置项占位）。
