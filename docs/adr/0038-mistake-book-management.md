# 0038 - 错题本管理命令与软删除

日期：2026-08-08
状态：已采纳

## 背景

错题本页需要支持整页编辑删除、长按/右键卡片菜单（追问、置顶、删除、取消置顶、已掌握）以及详情页标记已掌握。现有 `grading::list` 只能读错题，`MistakeStore` 的 `remove` 是物理删除，缺少置顶和批量删除能力。

## 决策

1. `Mistake` 增加两个轻量状态字段：
   - `pinned: bool`：置顶标记，`serde(default)` 兼容旧 `mistakes.json`。
   - `deleted_at: Option<DateTime<Utc>>`：软删除时间，非空表示已删除；`serde(default)` 兼容旧数据。
2. “已掌握”复用现有 `is_correct` 字段，不新增 `mastered` 字段，避免与 `report::weekly`、`tracking::checkin`、`exam::compose` 的判断逻辑分叉。
3. `MistakePatch` 扩展为完整可编辑字段：`subject`、`knowledge_point`、`question`、`student_answer`、`reference_answer`、`analysis`、`is_correct`、`pinned`。
4. `MistakeStore` 增加 `remove_many`；`remove` 与 `remove_many` 只写 `deleted_at`，不做物理删除；`get` 与 `list` 默认不返回已删除记录。
5. `grading` 插件新增四个 UserOnly Command：
   - `grading::get`：按 id 获取单条详情。
   - `grading::update`：单题编辑、置顶/取消置顶、标记已掌握。
   - `grading::remove`：单题软删除。
   - `grading::remove_many`：批量/全选软删除。
6. “追问”不新增后端命令，由前端跳转聊天页后调用现有 `send_user_message` 完成。

## 后果

- 错题本数据仍完整保留在 `mistakes.json`，后续可扩展恢复能力。
- 所有新命令继续经 `trigger_command` 唯一命令通道，自动落入现有 `EntryPointCall` 审计。
- 前端拿到 `pinned` 后可显示置顶卡片，拿到 `is_correct` 后可显示“已掌握”状态色。
- 旧错题数据反序列化时自动补默认值，不需要迁移文件。
