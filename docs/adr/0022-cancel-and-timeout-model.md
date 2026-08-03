# 两级取消与三层超时模型

工具执行采用两级取消，对齐操作系统信号语义：SIGTERM（合作式，AbortSignal/CancellationToken 传入 handler，宽限期默认 5s 内自主收尾）与 SIGKILL（强制式，dispatch 掐掉 handler 任务 JoinHandle，不给收尾机会）。用户取消与内部中断都走同一取消链；超时到期直接 SIGKILL（硬闹钟响了不再等收尾）。

超时是三层预算：工具级（ToolDef.timeout 声明，默认 30s，上限受回合预算钳制）、回合级（单轮总预算默认 10min，防连环失控）、活性超时（流式调用 60s 无增量即断，thinking 模式有增量就一直等）。工具可通过 DeadlineHandle::extend 申请延期（OCR 逐页调视觉模型时按页申请），每次延期受剩余回合预算钳制并进审计——内核兜底，插件不能无限延。

OCR 页级失败语义：先重试（默认 2 次，共 3 次尝试）；页级瞬时错误（解码/超时/限流）重试后仍失败则记入结果继续下一页；API 侧系统性错误（模型下架/余额不足/鉴权失败，ModelError 的 systemic 变体）直接中断整个工具调用（ToolError::model_unavailable, retryable=false），不白跑剩余页。
