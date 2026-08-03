# 单 agent 事件驱动 loop，v2 不做子 agent

kernel 采用单 agent、单 loop 的回合模型：LLM 是唯一决策者，kernel 执行工具调用并保证安全边界；loop 全程向 GUI 输出事件流（消息增量、工具开始/结束、回合结束）。LLM 通过统一的 ModelRuntime 抽象接入，v2 支持 OpenAI 兼容端点与 Ollama。子 agent 编排不进入 v2，列入后续优化；当前业务流程由模型在同一 loop 内连续调用工具完成。
