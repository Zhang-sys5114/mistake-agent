//! 用户插件聚合入口（ADR-0036）：清单由 build.rs 扫描 `src/plugin/*/mod.rs` 生成，
//! 插件开发者只需新建目录 + 写 mod.rs（实现 UserPlugin + descriptor），无需改本文件。
//! 规则见 docs/plugin-dev/user.md；参考模板见 docs/plugin-dev/reference/user-plugin/。

include!(concat!(env!("OUT_DIR"), "/builtin_user_plugins.rs"));

#[cfg(test)]
mod tests {
    // 编译锚定：参考模板必须始终与真实契约一致（不注册，仅编译检查）。
    include!("../docs/plugin-dev/reference/user-plugin/mod.rs");

    #[test]
    fn user_plugin_reference_typechecks() {
        let _ = descriptor();
    }
}
