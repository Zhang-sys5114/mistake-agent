# 内核插件开发手册

面向「写内核插件（特权子系统）」的开发者。内核插件运行在**信任边界内**，负责敏感资源与能力（会话存储、记忆、模型、验算、会话切换）；与用户插件的区别是：注册上下文注入**全量**服务句柄，并在 `info` 里用 `provides` 声明自己提供的服务。

内核整体生命周期、模块职责和扩展边界见 [Kernel 开发手册](../kernel-dev.md)；本文只讲 KernelPlugin 的注册与插件实现。

## 1. 快速开始（三步）

1. 复制参考模板：把 `reference/kernel-plugin/` 整个目录复制到 `src/kernel/plugin/<你的插件名>/`（目录名 = 模块名，小写蛇形）；
2. 编辑 `mod.rs`：改 `namespace`，如需提供服务则声明 `provides`（ServiceId），实现工具/命令/事件；
3. 构建即自动收录：`cargo check` / `cargo build`，无需改 `src/kernel/plugin/mod.rs`（ADR-0036）。

## 2. 目录形态与规则

- 一个插件一个文件夹：`src/kernel/plugin/<name>/`，入口必须是 `mod.rs`（如 `storage/` 内部再拆 `core.rs`/`file.rs`/`mem.rs`）。
- **职责先行**：开发时先规划职责；预计有两个及以上职责时，直接在插件目录下创建同名子文件夹与 `mod.rs`，不要先堆进单文件再拆。`mod.rs` 只负责公共面、装配和 `pub use` 重导出，职责实现放子模块；~400 行只是审查预警线，不是拆分触发条件。
- **目录即插件**：build.rs 扫描一层子目录里存在 `mod.rs` 的文件夹即收录；`services/` 是公共契约目录，不是插件。
- **禁用/WIP**：目录根部放一个空文件 `disabled`，整目录不编译、不注册；删掉即恢复。
- 目录名须为小写蛇形且非 Rust 关键字（目录名即模块名），违规构建期报错。

## 3. 两段式契约（KernelPlugin）

```rust
pub struct MyKernelPlugin;
impl KernelPlugin for MyKernelPlugin {
    fn info() -> Info { /* namespace + provides + 入口点 */ }
    fn register(ctx: KernelContext<'_>) -> Result<(), PluginError> { /* 绑定 handler */ }
}
pub fn descriptor() -> KernelDescriptor {
    KernelDescriptor::from_plugin::<MyKernelPlugin>()
}
```

### `Info` 差异（与用户插件相比）

| 字段 | 说明 |
|---|---|
| `namespace` | 全局唯一（与用户插件同一张表，互相抢占会被拒） |
| `provides` | 声明提供的服务；**每个 ServiceId 至多由一个内核插件提供**（重复 → `ServiceTaken`）。`session` 这类调度模块不占 ServiceId（provides 为空） |
| `load` | Eager / Lazy（默认 Lazy） |
| `tools/commands/events` | 同用户插件 |

用户插件禁止声明 `provides`（`ProvisionNotAllowed`）；内核插件不填 `requires`（全量句柄，requires 对它无意义）。

## 4. KernelContext：全量服务句柄

`ctx.handles` 直接给全量（storage/memory/compute/model 四个句柄），不做过滤——内核插件在信任边界内，是服务的提供者。仍经 `ctx.registrar` 绑定 handler，声明与实现一致约束不变。

服务实例本身**不**由 register 构造：storage/memory/model/compute 的实例在 `Kernel::new` 引导阶段按依赖顺序组装（数据根目录、settings 热更新、启动回退），注册表只负责身份、元数据与入口校验。你的内核插件若提供新服务，需要在 `services/` 增加契约/句柄，并在 `Kernel::new` 装配（这是目前唯一的手工接线点）。

## 4.5 磁盘 IO 铁律（ADR-0042）

**用户插件的一切磁盘读写只经 StorageHandle，不持有文件句柄**；内核插件（信任边界内）自管持久化目录，但**必须**经 storage 引出的文件能力，不直接裸 `std::fs`：

| 能力 | 位置 | 用途 |
|---|---|---|
| `DomainIo`（`read/write/remove/remove_tree/list`） | `services/storage.rs`，经构造注入（如 `FileMemoryService::new(io)`） | 数据根目录域内文件（`Domain` 枚举：mistakes/sessions/memory/data/uploads）；域根校验 + canonicalize 兜底 + 原子写 + 审计全在 storage 实现内 |
| `TmpIo`（`read_staged/remove_staged`） | `services/storage.rs`，经构造注入 | 系统 temp 暂存（`mistake-agent-` 前缀白名单，硬编码 `temp_dir()`）；读删都记审计 |

- **路径安全靠类型**：`RelPath::parse` 构造即校验（段白名单 `[a-zA-Z0-9._-]`、首尾必须字母数字、拒绝 `.`/`..`/`\`/`:`/非 ASCII），**不做任何规范化**，fail-closed——类型上不可能表示目录遍历。用户插件连 `RelPath` 都不用碰，只见 `StorageHandle` 语义方法（`read_staged/remove_staged/read_data_file/write_data_file`）。
- **存储布局迁移**：旧布局（中文路径落盘）→ 新布局（base64url 段编码）经 `DomainIo::read_legacy/remove_legacy` 通道迁移（仅启动引导用，允许非 ASCII 段但拒绝遍历向量 + canonicalize 兜底），样例见 `FileMemoryService::migrate_legacy_layout`。
- **审计**：每次域内/暂存 IO 都记 `AuditRecord::FileIo` / `StagedFileIo`（调 trait = 自动审计，无逃逸点）。
- **样例**：memory 的 `FileMemoryService` 已收编（`memory/store.rs`）——中文记忆路径经 base64url 段编码落盘（RelPath 白名单是 ASCII，编码后天然满足），列出时解码还原。
- **例外**：`verify_geometry.py` 是执行代码不是数据，维持编译期 include_str!；GUI 壳（main.rs）不是插件，保留自有 canonicalize 白名单。

## 5. 真实示例

- `memory`：`save/show/remove` 工具入口（服务实现在同一文件夹）；
- `compute`：`verify` 工具（BridgeCompute → GUI Pyodide）；
- `session`：`switch` 工具（工具入口与 Session scheduler 分离，调度器在 `src/kernel/agent/session/`）；
- `storage` / `model`：纯服务提供者，无工具入口，`register` 为空。

## 6. 注册校验与常见错误

与用户插件同表：namespace 唯一 → provides 唯一 → 入口名不重复 → wire name 全局唯一。常见错误：

- `ServiceTaken`：两个内核插件声明了同一个 ServiceId；
- `NamespaceTaken` / `WireNameCollision`：与用户插件也会互相撞车；
- `UndeclaredEntry`：register 登记了 info 没声明的短名；
- 目录名违规或缺 `descriptor()`：构建期报错（前者 build.rs 点名目录，后者编译期兜住）。

## 7. 参考

- 参考模板：[reference/kernel-plugin/](./reference/kernel-plugin/mod.rs)（有编译锚定测试）；
- 用户插件（业务侧）写法：[user.md](./user.md)；
- 公共契约与句柄：`src/kernel/plugin/services/`；注册表：`src/kernel/registry/`。
