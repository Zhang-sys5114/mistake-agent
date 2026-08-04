# 记忆文件持久化与事件/审计接入

memory 服务从内存实现升级为文件持久化：数据根目录 `memory/<路径>.md`（ADR-0011 布局）。目录语义与 Q9 契约一致：`show(None)` 递归列出全部条目（不带 `.md` 后缀、按路径排序），`show(Some(p))` 读取单条，`remove(p)` 删除条目及其子树（路径前缀匹配）。`InMemoryMemory` 保留作测试与启动回退。

路径校验收紧：除拒绝空段、`..`、反斜杠与控制字符外，拒绝 `.` 段，杜绝"当前目录"歧义。

MemoryHandle 注入 EventSink 与 Auditor：save/remove 成功时发 `Event::MemoryChanged` 并记录 `MemoryWrite`/`MemoryRemove`，show 记录 `MemoryRead`（读取不播事件，事件语义是"变更"）。构造函数变化只影响 Kernel 组装，插件可见接口不变。

文件写入采用临时文件 + rename 原子替换；删除只作用于 `memory/` 子树，绝不触碰数据根目录之外。
