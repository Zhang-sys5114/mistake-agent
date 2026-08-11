# 错题本目录化存储与事件流

决策：错题本从单文件 `mistakes/mistakes.json` 改为**每道错题一个目录**（错题条目）：`mistakes/<id>/` 内含 `mistake.json`（当前快照）、`events.jsonl`（该题事件流）、`schedule.json`（掌握度调度，见 ADR-0040）。判分与掌握度变更以**纯追加事件**落 `events.jsonl`（`graded`：对错/来源/时长，`mastery_changed`：掌握翻转），是「正确率变化、反复丢分、掌握度」等时间线统计的唯一业务真相；`is_correct` 只是事件折叠出的最新快照。旧 `mistakes.json` 由 bootstrap 启动引导一次性迁移（逐题原子拆 + 幂等跳过已拆题 + 完成后旧文件改名 `.bak`，每条旧题回填一条 `source=backfill` 的 graded 事件补齐历史时间线）。

动机：单文件全量重写随题量增长写放大；事件内嵌记录会被 `is_correct` 覆盖写毁掉历史；场景 3（复盘）与场景 5（追踪）的统计全部依赖可回放的时间线。目录化与 `sessions/<key>.jsonl` 每会话一文件的既有哲学一致。

与审计的边界：审计（Audit）是操作事实记录（10MB 轮转、大内容引用），事件流是业务时间线（不轮转、只追加）——两者不合并；事件写入由 storage 内核插件经 `StorageHandle` 提供，用户插件只调接口，不破信任模型。

考虑过的替代方案：事件内嵌 `Mistake.attempts` 数组（被否：快照与时间线耦合，编辑/迁移语义混乱）；单文件共享事件流 `mistakes/events.jsonl`（被否：写局部性差、与目录化哲学不一致）。
