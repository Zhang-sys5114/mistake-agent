# 0035 — 内核插件采用与用户插件一致的两段式注册（KernelPlugin）

日期：2026-08-05
状态：已采纳

## 背景

ADR-0021 曾定：内核插件（memory/compute/session）的工具入口由 kernel 启动时**直连注册**，不走用户插件两段式契约，但与用户插件同表校验。落地时这几处入口实际以用户插件形式写在 `src/plugin/`（memory/compute/session 各有 descriptor），四个内核服务（storage/memory/compute/model）的构造与装配则硬编码在 `Kernel::new`——内核模块自身没有任何注册入口，注册机制与"内核插件"的身份不一致：

- 工具入口的身份是"用户插件"，而非其真正归属的内核模块；
- 服务提供者（storage/model）无注册信息，namespace 可被用户插件抢占；
- ServiceId 提供关系无启动期校验（两个模块同时提供同一服务不会被发现）。

## 决策

1. **新增 `KernelPlugin` 两段式契约**（`info()` + `register(ctx)`），形状与 `UserPlugin` 完全一致；`Info` 增加 `provides: Vec<ServiceId>` 字段——内核插件在 info 中声明其提供的 ServiceId，用户插件不得声明（fail-fast 拒绝）。
2. **同一张注册表**：内核插件经 `Registry::register_kernel_plugin` 注册，与用户插件共用 namespace 唯一、wire name 全局唯一、CallerPolicy、懒/急加载（LoadPolicy）全部校验与语义；跨用户/内核插件的 wire 撞名同样被拒绝。
3. **注册上下文差异**：`KernelContext` 注入**全量** `ServiceHandles`（内核插件在信任边界内，是服务提供者，不做 requires 过滤）；用户插件仍只拿到 requires 声明的受限句柄。
4. **入口归属回归内核模块**：`memory::save/show/remove`、`compute::verify`、`session::switch` 从 `src/plugin/` 移入各自内核模块（`src/kernel/plugin/memory/`、`src/kernel/plugin/compute/`、`src/kernel/plugin/session/`；Session scheduler 留在 `src/kernel/agent/session/`），以 `KernelPlugin` 注册；storage/model 以内核插件身份声明 provides（当前无工具入口）。聚合清单 `kernel::plugin::builtin_kernel_plugins()` 与 `plugin::builtin_plugins()` 并列。
5. **服务实例仍由 `Kernel::new` 引导构造**（依赖数据根目录、settings 热更新与启动回退策略），注册表负责身份/入口校验，不接管服务生命周期。

备选方案：

- 内核插件另建独立注册表（否决：违反"同表校验"，两份撞名逻辑重复）；
- 保持现状、工具入口继续以用户插件注册（否决：身份错位，storage/model 无注册信息，ServiceId 提供无校验）；
- 服务构造一并搬进 register（否决：storage → auditor → memory/model/compute 存在依赖顺序，register 无返回值通道，会引入两阶段服务注册复杂度，v2 封闭集合内收益低）。

## 后果

- `src/plugin/` 只保留业务用户插件（hello/grading/practice/report/exam/tracking，6 个）；`src/kernel/` 五个模块各带 `descriptor()`。
- 用户可见性不变：memory/compute 工具仍出现在用户功能中心与模型工具列表（同表注册，`user_entries`/`model_tools` 不区分来源）；`session::switch` 仍由 agent loop 特殊执行。
- 新增校验：ServiceId 提供唯一（`ServiceTaken`）、用户插件不得声明 provides（`ProvisionNotAllowed`）。
- 测试：注册表新增内核插件懒加载、provides 去重、跨用户/内核 wire 撞名用例；原 plugin/memory、plugin/compute 的 handler 测试随代码迁移。
