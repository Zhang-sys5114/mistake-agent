# 服务契约角色拆分与受控句柄注入

v2 的四个内核服务（storage/memory/compute/model）在契约层按角色拆分：storage 拆成 SessionStore（kernel 内部）、MistakeStore（用户插件可见）、AuditSink（kernel 内部）三个角色 trait，由同一实现组合成 StorageService；memory/model/compute 各一个角色 trait。ServiceHandles 是类型化封闭容器（四个字段），注入给插件的是受控视图（StorageHandle 只暴露错题本五个操作、ModelHandle 只暴露带超时/abort/审计的 complete），过滤由结构保证而不是运行时检查。kernel 内部持有全量 trait 引用（loop 用 ModelService 流式、scheduler 用 SessionStore、Auditor 用 AuditSink）。另定：内核插件（memory/compute/session）的工具入口由 kernel 启动时直连注册，不走用户插件两段式契约，但与用户插件同表校验（namespace/wire 唯一、CallerPolicy）。

选型理由：三个消费方（会话调度、用户插件、审计）需求完全不同，一个 god trait 会让插件"看得见摸不着"的方法成为诱惑；角色拆分让"加能力 = 加角色 trait"（加法运算），受控句柄让权限边界编译期可见。备选的 HashMap<ServiceId, Arc<dyn Any>> 容器被否决：v2 服务集合封闭（ADR-0014），类型化容器更自文档、无运行时 downcast。
