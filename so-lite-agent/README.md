# so-lite-agent

通用轻量 Agent 运行时：agent loop、工具注册/调度、消息树、事件流、会话存储抽象、模型 Provider 抽象与通用 RPC，`cargo add` 后即可开发自己的 Agent。

> 状态：M2 骨架（对应 [docs/plan/so-lite-agent.md](../docs/plan/so-lite-agent.md)）。默认服务为 `InMemorySessionStore` 与 `MockModelService`；Provider 适配器、真实 API 回合、插件手册迁移与 crates.io 发布为 M3-M5 待办。

## 快速验证

```bash
cargo run --example hello
cargo test
```

`examples/hello.rs` 注册一个 `hello::hi` 用户插件，用默认 mock 模型跑通一个完整回合。

## 最小用法

```rust
let kernel = KernelBuilder::new()
    .event_sink(events)
    .register_plugin(my_business_plugin())
    .system_prompt(|| agent_system_prompt())
    .build()
    .await?;
```

内核插件与用户插件由使用方编写；业务服务（存储、记忆、验算、双模型等）不随本 crate 分发。
