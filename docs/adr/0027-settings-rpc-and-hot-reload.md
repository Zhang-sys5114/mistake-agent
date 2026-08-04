# settings RPC 与运行时热更新

RPC 接通 `get_settings`/`set_settings`（ADR-0013 方法清单）：`get_settings` 返回 public_view（仅 `key_set` 布尔，绝不返回 api_key）；`set_settings` 校验（api_url 必须 http(s)、模型名非空）、原子写 settings.json、返回新 public_view。

保存成功后：模型服务经 `LiveSettingsModelService.refresh()` 立即按新配置重建适配器（下一次模型调用即生效），日志级别经 flexi_logger 句柄热切换（不生效时下次启动兜底），并投递 `SettingsChanged` 中断 + 审计。

`LiveSettingsModelService` 持有共享 `RwLock<Settings>`，按 ModelKind 重建底层适配器；不刷新时行为与构建期快照一致。明文 key 取舍维持 ADR-0015（DPAPI 列后续）。
