# 0036 — 构建期自动发现插件清单（build.rs 生成，替代手写聚合）

日期：2026-08-05
状态：已采纳

## 背景

ADR-0002 定稿编译期内置插件、无动态加载；ADR-0035 后用户插件与内核插件统一两段式契约。但每个插件仍需在 `src/plugin.rs`（用户）或 `src/kernel/plugin/mod.rs`（内核）手写两处登记（`pub mod` + `descriptor()`），新增/改名/禁用都要改聚合文件——与「插件开发者只动自己的插件文件夹」的目标冲突。另：build.rs 在编译前运行、读不到代码里的 `Info` 值，运行时 enabled 字段无法实现构建期跳过。

## 决策

1. **目录即插件**：build.rs 扫描 `src/plugin/*/mod.rs` 与 `src/kernel/plugin/*/mod.rs`（一层深、字母序），生成 `builtin_user_plugins.rs` / `builtin_kernel_plugins.rs` 到 OUT_DIR，经 `include!` 引入；`src/plugin.rs` 与 `src/kernel/plugin/mod.rs` 退化为壳。`cargo:rerun-if-changed` 监听两个目录，目录一变化自动重扫。
2. **禁用 = `disabled` 空文件**：插件目录根部存在 `disabled` 文件则整目录跳过——不编译、不注册；删除即恢复。不在 `Info` 引入 enabled 字段：build.rs 读不到代码值，单一事实源是文件存在性。
3. **命名校验**：目录名即模块名，须匹配 `^[a-z][a-z0-9_]*$` 且非 Rust 关键字，违规构建期报错并点名目录；缺 `descriptor()` 由编译期报错兜住（不做文本扫描）。
4. **include! 内 mod 解析**：生成清单里的模块声明带绝对 `#[path]`（构建期按机器生成、正斜杠归一化），解决 include! 内联后 `mod` 按包含文件目录解析的问题。
5. **参考模板编译锚定**：`docs/plugin-dev/reference/{user,kernel}-plugin/` 经 `include!` 引入测试模块做编译检查，模板与真实契约同步校验。

## 后果

- 新增插件 = 建目录 + 写 mod.rs（实现 UserPlugin/KernelPlugin + descriptor()），聚合文件零改动；
- 停用/WIP = 放 `disabled` 文件，目录不参与编译（半成品可安全停放）；
- 清单顺序改为字母序（懒加载插件顺序无关，session 急加载不受影响）；
- 插件开发手册（docs/plugin-dev/）与参考模板成为唯一的「怎么写插件」入口。

## 备选

- 维持手写聚合（否决：违背目录即插件）；
- `Info.enabled` 运行时过滤（否决：build.rs 读不到代码值，无法构建期跳过，半成品仍炸构建）；
- `_` 前缀跳过（否决：用户要求显式标记，空文件比命名约定更醒目）；
- 文本扫描 mod.rs 找 enabled 字样（否决：脆，注释误命中、间接写法漏命中）。
