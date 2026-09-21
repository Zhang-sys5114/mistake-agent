# 内部中断：InterruptBus 环境变更信号

> 注（ADR-0044 修订）：会话调度不再是中断生产者——`SessionSwitched` / `GoalUpdated` 已随模型自动切换一并删除，生产者收敛为 settings（`ConfigChanged`）、memory（`MemoryChanged`）、compaction（`CompactionDone`）。空闲超时提示改走面向 GUI 的 `Event::SessionIdle`（Event 通道，见末句"两个通道"）。

参考单片机中断系统：外部中断 = 用户/GUI 事件（abort、新消息），内部中断 = 内核组件主动向 agent loop 发出的环境变更信号。InterruptBus 是内核级队列（VecDeque），生产者包括 settings（ConfigChanged）、memory（MemoryChanged）、compaction（CompactionDone）；agent loop 在回合边界消费（take_all），据此重组下回合上下文。真正需要立刻打断的场景不走队列，直接走 Q12 取消链（如设置中途变更 → 中断当前回合）；不做中断优先级：消费点在回合边界天然有序，需要优先的场景本来就该用取消链。内部中断与面向 GUI 的 Event 播报是两个通道（一个消费、一个展示），同一事实可同时发两者。
