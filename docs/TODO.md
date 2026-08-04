# TODO

## OOBE 初始化数据根目录（未完成）

现状：OOBE 首次向导只调用 `set_settings` 保存模型配置（[web/src/components/OobePage.vue](../web/src/components/OobePage.vue)）；数据根目录 `~/Documents/.mistake-agent/` 下的目录与文件由各模块首次使用时懒创建（storage、memory、logger、uploads），`AGENTS.md`（教学规则）目前没有任何初始化逻辑。

目标：首次向导完成时（或 kernel 启动引导阶段）一次性初始化：

- 创建数据根目录及子目录：`sessions/`、`mistakes/`、`memory/`、`audit/`、`logs/`、`uploads/`
- 若 `AGENTS.md` 不存在，写入默认模板（教学规则占位，家长/老师可编辑；已存在则不覆盖）
- 初始化归 kernel 负责（建议新增内核引导逻辑或 RPC，前端不直接写文件系统），与 `set_settings` 保存一同在 OOBE 完成路径触发

参考：PROJECT.md §6 数据与配置；[src/kernel/settings.rs](../src/kernel/settings.rs) `data_root()`。
