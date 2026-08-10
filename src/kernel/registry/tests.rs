use super::*;

use serde_json::{Value, json};

use crate::kernel::agent::dispatch::ToolCallContext;

use crate::kernel::logger::Logger;

fn logger() -> LoggerHandle {
    Arc::new(Logger)
}

#[test]
fn duplicate_namespace_rejected() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_plugin(desc).unwrap();
    let desc2 = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(desc2),
        Err(PluginError::NamespaceTaken(_))
    ));
}

#[test]
fn wire_name_mapping_separates_single_underscores() {
    // 双下划线映射（ADR-0020）：a::b_c → a__b_c，a_b::c → a_b__c，不再撞名。
    assert_eq!(full_to_wire("a::b_c"), "a__b_c");
    assert_eq!(full_to_wire("a_b::c"), "a_b__c");

    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "a".into(),
            tools: vec![tool_def("b_c", "t1", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_plugin(desc).unwrap();
    let desc2 = PluginDescriptor {
        info: Info {
            namespace: "a_b".into(),
            tools: vec![tool_def("c", "t2", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_plugin(desc2).unwrap();
}

#[test]
fn double_underscore_wire_collision_rejected() {
    // 病态组合仍撞：a::b__c → a__b__c 与 a__b::c → a__b__c，注册期全局校验兜底。
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "a".into(),
            tools: vec![tool_def("b__c", "t1", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_plugin(desc).unwrap();
    let desc2 = PluginDescriptor {
        info: Info {
            namespace: "a__b".into(),
            tools: vec![tool_def("c", "t2", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(desc2),
        Err(PluginError::WireNameCollision(_))
    ));
}

#[test]
fn requires_must_be_satisfiable() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            requires: vec![ServiceId::Model],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(desc),
        Err(PluginError::CapabilityUnavailable(_))
    ));
}

#[test]
fn lazy_registration_on_first_use() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            tools: vec![tool_def("hello", "打招呼", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |ctx| {
            ctx.registrar.tool(
                "hello",
                std::sync::Arc::new(|_ctx: &ToolCallContext, _p: Value| {
                    Box::pin(async move { Ok(json!({"reply": "hi"})) })
                }),
            )
        },
    };
    registry.register_plugin(desc).unwrap();
    assert!(registry.handlers.read().unwrap().is_empty());
    let entry = registry.ensure_tool("demo::hello").unwrap();
    assert_eq!(entry.full_name, "demo::hello");
    assert_eq!(registry.model_tools().len(), 1);
    assert_eq!(registry.model_tools()[0].name, "demo__hello");
    // 重复注册被拒
    let desc2 = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            tools: vec![tool_def("hello", "x", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(desc2),
        Err(PluginError::NamespaceTaken(_))
    ));
}

#[test]
fn lazy_wire_resolution_loads_plugin_on_first_call() {
    // 模型走 wire 名：未加载插件也能被解析（按 info 声明反查 → 懒加载），
    // 解决「模型列表看不到懒插件 → 猜名调用 → unknown_tool」的问题。
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            tools: vec![tool_def("hello", "x", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |ctx| {
            ctx.registrar.tool(
                "hello",
                std::sync::Arc::new(|_ctx: &ToolCallContext, _p: Value| {
                    Box::pin(async move { Ok(json!({"reply": "hi"})) })
                }),
            )
        },
    };
    registry.register_plugin(desc).unwrap();
    assert!(registry.handlers.read().unwrap().is_empty());
    // 未加载插件的声明工具仍在模型列表（懒加载发生在调用时）。
    assert_eq!(registry.model_tools().len(), 1);
    assert_eq!(registry.model_tools()[0].name, "demo__hello");
    let full = registry
        .resolve_wire("demo__hello")
        .expect("懒加载后应可解析");
    assert_eq!(full, "demo::hello");
    assert_eq!(registry.handlers.read().unwrap().len(), 1);
}

#[test]
fn user_entries_filter_invisible() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            tools: vec![
                ToolDef {
                    name: "hidden".into(),
                    user_visible: false,
                    title: None,
                    group: None,
                    description: "模型专用".into(),
                    params: crate::kernel::contract::empty_params(),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: None,
                },
                ToolDef {
                    name: "shown".into(),
                    user_visible: true,
                    title: Some("可见工具".into()),
                    group: Some("测试".into()),
                    description: "用户可用".into(),
                    params: crate::kernel::contract::empty_params(),
                    policy: CallerPolicy::UserAndModel,
                    timeout: None,
                    icon: None,
                },
            ],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_plugin(desc).unwrap();
    let entries = registry.user_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["entry"], "demo::shown");
    assert_eq!(entries[0]["title"], "可见工具");
    assert_eq!(entries[0]["group"], "测试");
}

#[test]
fn kernel_plugin_lazy_registration_binds_handler() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = KernelDescriptor {
        info: Info {
            namespace: "kernel_demo".into(),
            provides: vec![ServiceId::Memory],
            tools: vec![tool_def("ping", "内核工具", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |ctx| {
            ctx.registrar.tool(
                "ping",
                std::sync::Arc::new(|_ctx: &ToolCallContext, _p: Value| {
                    Box::pin(async move { Ok(json!({"kernel": true})) })
                }),
            )
        },
    };
    registry.register_kernel_plugin(desc).unwrap();
    // 懒加载：注册后 handler 表为空，首次 ensure 才绑定。
    assert!(registry.handlers.read().unwrap().is_empty());
    let entry = registry.ensure_tool("kernel_demo::ping").unwrap();
    assert_eq!(entry.full_name, "kernel_demo::ping");
    assert_eq!(registry.model_tools().len(), 1);
    assert_eq!(registry.model_tools()[0].name, "kernel_demo__ping");
}

#[test]
fn duplicate_service_provision_rejected() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = KernelDescriptor {
        info: Info {
            namespace: "a".into(),
            provides: vec![ServiceId::Memory],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_kernel_plugin(desc).unwrap();
    let desc2 = KernelDescriptor {
        info: Info {
            namespace: "b".into(),
            provides: vec![ServiceId::Memory],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_kernel_plugin(desc2),
        Err(PluginError::ServiceTaken(ServiceId::Memory))
    ));
}

#[test]
fn user_plugin_cannot_declare_provides() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    let desc = PluginDescriptor {
        info: Info {
            namespace: "demo".into(),
            provides: vec![ServiceId::Storage],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(desc),
        Err(PluginError::ProvisionNotAllowed(_))
    ));
}

#[test]
fn kernel_and_user_wire_collision_rejected() {
    let registry = Registry::new(ServiceHandles::default(), logger());
    // 内核插件 a::b__c 与用户插件 a__b::c 共享 wire a__b__c（病态组合，仍全局唯一校验）。
    let kernel = KernelDescriptor {
        info: Info {
            namespace: "a".into(),
            tools: vec![tool_def("b__c", "t1", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    registry.register_kernel_plugin(kernel).unwrap();
    let user = PluginDescriptor {
        info: Info {
            namespace: "a__b".into(),
            tools: vec![tool_def("c", "t2", CallerPolicy::UserAndModel)],
            ..Default::default()
        },
        register: |_| Ok(()),
    };
    assert!(matches!(
        registry.register_plugin(user),
        Err(PluginError::WireNameCollision(_))
    ));
}
