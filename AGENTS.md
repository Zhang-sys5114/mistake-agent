# AGENTS.md

## 项目速览

Mistake Agent v2：面向中学生的本地错题管理 + 辅助学习 Agent（Windows 桌面应用，Tauri GUI + 自研 Rust kernel，Rust 2024 edition，mistake-agent 本体单 crate；ADR-0037 的 `so-lite-agent/` 为独立 crate 骨架）。

**动任何代码之前，先读一遍 [PROJECT.md](PROJECT.md)**——它是唯一入门文档，包含架构、信任模型、机制、命名规范、里程碑和分工。

## 读文档路由：做什么 → 读什么

| 你要做什么 | 先去读 | 重点内容 |
|---|---|---|
| 刚加入项目 | PROJECT.md 全文 + CONTEXT.md | 全貌、术语；不懂的词去 CONTEXT.md 查 |
| 改设计 / 做架构决策 | docs/adr/ 全部 + CONTEXT.md | 决策留痕；新决策要新增 ADR 并更新术语 |
| 看后续计划 / so-lite-agent 剥离 | docs/plan/so-lite-agent.md + docs/adr/0037 | M1/M2 已落地，M3-M5 待办；mistake-agent 本体在 M5 前仍单 crate |
| 改内核机制（loop/调度/注册表） | PROJECT.md §4-§5 + docs/adr/0003~0010 | 两段式契约、CallerPolicy、护栏、容灾 |
| 改会话 / 消息树 | PROJECT.md §5 会话 + docs/adr/0006、0007 | 双层调度、守卫模型、Goal、历史路由 |
| 改内核插件（services） | PROJECT.md §4-§5 + docs/adr/0001、0014、0015、0016 | 服务句柄、ModelHandle、配置独占、compute 桥接 |
| 改记忆 | PROJECT.md §5 记忆 + docs/adr/0008 | memory::save/show/remove、路径校验、路由式浏览 |
| 改用户插件（plugin/） | PROJECT.md §3、§12 + docs/adr/0002、0003 | 五场景、命名规范、入口点、注册校验 |
| 改 GUI / 协议 | PROJECT.md §5 通信 + docs/adr/0013 | trigger_command 唯一命令通道、事件流 |
| 改模型 / 设置 | PROJECT.md §6 + docs/adr/0015、0019 | 双模型配置、用户独占写、明文 key 取舍 |
| 改审计 / 日志 | PROJECT.md §5 审计日志 + docs/adr/0017、0018 | 全覆盖审计、分级日志、脱敏 |
| 抄开源代码 | 该项目 LICENSE + PROJECT.md §2 开源策略 | 保留许可声明、注明来源；机制可抄，业务自写 |
| 写测试 | PROJECT.md §10 里程碑验收标准 + 各模块 tests | 按验收标准补测试 |

## 常用命令

```bash
cargo check        # 快速检查
cargo test         # 单元测试
cargo clippy -- -D warnings
cargo fmt --check
```

## 架构红线（改代码时逐条遵守）

- mistake-agent 本体单 crate：`src/kernel/`（内核）与 `src/plugin/`（用户插件）分区，**M5 切换前不再新增 crate 拆分**；`so-lite-agent/` 是 ADR-0037 允许的独立通用运行时 crate（M1/M2 已落地，M3-M5 待办）
- 能力边界：内核实现用 `pub(crate)` 隐藏；用户插件只经公开 API 面交互；不引入全局可变状态绕过句柄
- CallerPolicy：`UserAndModel` 工具必须配同名用户入口；`UserOnly` 不得进入模型工具列表
- 入口点命名 `namespace::tool`：插件只写短名，kernel 拼全名，撞名由注册表拒绝
- 审计默认全覆盖、日志分级、敏感值（API key 等）脱敏
- 模型：主模型 deepseek-v4-flash；视觉模型 qwen3-VL（SiliconFlow）；settings 配 API_URL/API_KEY
- 抄开源代码必须保留原许可证声明并在文档注明来源

## 开发约定

- 提交信息用简洁中文描述（如 `feat(kernel): 注册表与 CallerPolicy 校验`）
- 改动必须通过 `cargo test` 与 `cargo clippy -- -D warnings`
- 改设计不留痕 = 没改：必须同步 CONTEXT.md 或新增 docs/adr/

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, accessed via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default labels (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`) map the five canonical triage roles. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: one `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
