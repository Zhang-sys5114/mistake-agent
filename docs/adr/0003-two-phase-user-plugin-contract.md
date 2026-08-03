# 用户插件采用两段式契约（info + register）

用户插件实现 `info()`（静态元数据：namespace、能力声明、工具/命令/事件定义）和 `register(ctx)`（注入服务句柄、绑定 handler）两个阶段。入口点共三类：Tool（LLM 调度）、Command（GUI/用户调度）、Event（kernel 生命周期调度）。kernel 启动时收集全部 `info()` 用于校验（namespace 唯一、服务依赖可满足）、生成 GUI 命令面板元数据和文档；`register()` 在实际需要时执行。`info()` 中的 LoadPolicy（默认 lazy）决定插件是读取即加载（eager）还是首次使用才加载（lazy）。备选的一步式注册（在 register 里自声明元数据）被否决：撞名检查会变晚，GUI 也必须实例化插件才能拿到命令列表。
