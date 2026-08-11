# 掌握度调度与裁决（Anki 式）

决策：每道错题维护 Anki 式掌握度调度（错题条目内 `schedule.json`：`interval/ease/due_at/last_result`），由判分事件折叠更新，调度层与裁决层分离：

- **调度层（节奏）**：错 1 次即重置间隔回 7 天（SM-2 again 语义）、`ease - 0.2`；答对 `interval × 2`、`due_at = now + interval`——重测节奏随表现动态调整，替代固定 7/14/30 天硬编码（初始档位仍兼容任务书）。
- **裁决层（掌握）**：`is_correct` 连错 2 次才打回 `false`（"反复丢分"语义，避免偶然失误误伤）；用户可手动标已掌握，但考卷达标是系统级证据（卷内该知识点题数 ≥2 且得分率 ≥80%）自动置 `true` 并记 `mastery_changed(source=exam_pass)`——防用户谎报已掌握，模型经判分管线拥有把控权而不直接触碰管理字段。

同时 `grading::update` 从 UserOnly Command 升级为 UserAndModel Tool（经 dispatch 命令回退机制，前端 `trigger_command` 零改动）：模型可改**内容字段**（subject/knowledge_point/question/student_answer/reference_answer/analysis），不可改**管理字段**（is_correct/pinned/deleted_at）；删除（remove/remove_many）与已掌握标记保持 UserOnly——模型幻觉写错内容可自愈，破坏性操作仅用户可做。

动机：用户自报已掌握无校验（撒谎即可绕过重测），模型是错题本主要写入者却无修正能力；Anki 式调度让"何时再考"有据可依且与 Anki 生态心智对齐。

考虑过的替代方案：两段式确认（用户申请→模型验证后才生效，被否：多状态字段 + 标了不生效体验差）；已掌握永不过期（被否：撒谎无代价）。
