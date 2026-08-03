//! 注册表（Q11）：启动 fail-fast 校验、两段式契约、懒注册、模型工具列表过滤。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use schemars::Schema;

use crate::kernel::context::{EntryRegistrar, PluginContext, RegistrarTargets};
use crate::kernel::contract::{
    CallerPolicy, Info, LoadPolicy, PluginError, ToolDef, full_name, full_to_wire,
};
use crate::kernel::dispatch::{CommandHandler, EventHandler, ToolHandler};
use crate::kernel::logger::LoggerHandle;
use crate::kernel::services::{ServiceHandles, ToolSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Tool,
    Command,
    Event,
}

#[derive(Clone)]
pub enum Handler {
    Tool(ToolHandler),
    Command(CommandHandler),
    Event(EventHandler),
}

#[derive(Clone)]
pub struct RegisteredEntry {
    pub full_name: String,
    pub kind: EntryKind,
    pub policy: CallerPolicy,
    pub timeout: Option<Duration>,
    pub description: String,
    pub icon: Option<String>,
    pub params: Schema,
    pub handler: Handler,
}

/// 编译期内置插件的静态描述符（ADR-0002）。
pub struct PluginDescriptor {
    pub info: Info,
    pub register: fn(PluginContext<'_>) -> Result<(), PluginError>,
}

impl PluginDescriptor {
    pub fn from_plugin<P: UserPlugin>() -> Self {
        Self {
            info: P::info(),
            register: P::register,
        }
    }
}

/// 用户插件两段式契约（ADR-0003）：info 静态元数据，register 绑定 handler。
pub trait UserPlugin {
    fn info() -> Info;
    fn register(ctx: PluginContext<'_>) -> Result<(), PluginError>;
}

struct PluginEntry {
    info: Info,
    register: fn(PluginContext<'_>) -> Result<(), PluginError>,
    loaded: AtomicBool,
}

pub struct Registry {
    entries: RwLock<HashMap<String, Arc<PluginEntry>>>,
    handlers: RwLock<HashMap<String, RegisteredEntry>>,
    wire_to_full: RwLock<HashMap<String, String>>,
    services: ServiceHandles,
    logger: LoggerHandle,
}

impl Registry {
    pub fn new(services: ServiceHandles, logger: LoggerHandle) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            handlers: RwLock::new(HashMap::new()),
            wire_to_full: RwLock::new(HashMap::new()),
            services,
            logger,
        }
    }

    pub fn logger(&self) -> &LoggerHandle {
        &self.logger
    }

    /// 注册插件：启动时 fail-fast 校验（Q11）。
    pub fn register_plugin(&self, desc: PluginDescriptor) -> Result<(), PluginError> {
        let info = desc.info;
        {
            let entries = self.entries.read().expect("registry poisoned");
            if entries.contains_key(&info.namespace) {
                return Err(PluginError::NamespaceTaken(info.namespace.clone()));
            }
        }

        let available = self.services.available();
        let missing: Vec<_> = info
            .requires
            .iter()
            .filter(|r| !available.contains(r))
            .copied()
            .collect();
        if !missing.is_empty() {
            return Err(PluginError::CapabilityUnavailable(missing));
        }

        let mut fulls: HashSet<String> = HashSet::new();
        let mut wires: HashSet<String> = HashSet::new();
        {
            // 跨已注册插件做 wire 全局唯一检查（Q7）。
            let entries = self.entries.read().expect("registry poisoned");
            for e in entries.values() {
                for t in &e.info.tools {
                    wires.insert(full_to_wire(&full_name(&e.info.namespace, &t.name)));
                }
                for c in &e.info.commands {
                    wires.insert(full_to_wire(&full_name(&e.info.namespace, &c.name)));
                }
                for ev in &e.info.events {
                    wires.insert(full_to_wire(&full_name(&e.info.namespace, &ev.name)));
                }
            }
        }
        let mut check_name = |short: &str, kind: &str| -> Result<(), PluginError> {
            let full = full_name(&info.namespace, short);
            let wire = full_to_wire(&full);
            if !fulls.insert(full.clone()) {
                return Err(PluginError::DuplicateEntry(full));
            }
            if !wires.insert(wire.clone()) {
                return Err(PluginError::WireNameCollision(format!(
                    "{kind} {short} → {wire}"
                )));
            }
            Ok(())
        };
        for t in &info.tools {
            check_name(&t.name, "工具")?;
        }
        for c in &info.commands {
            check_name(&c.name, "命令")?;
        }
        for e in &info.events {
            check_name(&e.name, "事件")?;
        }

        let eager = matches!(info.load, LoadPolicy::Eager);
        let entry = Arc::new(PluginEntry {
            info,
            register: desc.register,
            loaded: AtomicBool::new(false),
        });
        self.entries
            .write()
            .expect("registry poisoned")
            .insert(entry.info.namespace.clone(), entry.clone());
        if eager {
            self.load_plugin(&entry.info.namespace)?;
        }
        Ok(())
    }

    /// 懒注册：首次命中某插件任一入口点时调用 register（Q4）。
    pub fn load_plugin(&self, namespace: &str) -> Result<(), PluginError> {
        let entry = {
            let entries = self.entries.read().expect("registry poisoned");
            entries
                .get(namespace)
                .cloned()
                .ok_or_else(|| PluginError::Internal(format!("未知插件：{namespace}")))?
        };
        if entry.loaded.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let targets = RegistrarTargets {
            handlers: &self.handlers,
            wire_to_full: &self.wire_to_full,
        };
        let registrar = EntryRegistrar::new(namespace, &entry.info, targets);
        let ctx = PluginContext {
            handles: self.services.filter(&entry.info.requires),
            logger: self.logger.clone(),
            registrar,
        };
        let result = (entry.register)(ctx);
        if result.is_err() {
            entry.loaded.store(false, Ordering::SeqCst);
        }
        result
    }

    pub fn ensure_tool(&self, full: &str) -> Result<RegisteredEntry, PluginError> {
        self.ensure(full, EntryKind::Tool)
    }

    pub fn ensure_command(&self, full: &str) -> Result<RegisteredEntry, PluginError> {
        self.ensure(full, EntryKind::Command)
    }

    fn ensure(&self, full: &str, kind: EntryKind) -> Result<RegisteredEntry, PluginError> {
        if !self
            .handlers
            .read()
            .expect("registry poisoned")
            .contains_key(full)
        {
            let ns = full.split("::").next().unwrap_or_default().to_string();
            self.load_plugin(&ns)?;
        }
        let entry = self
            .handlers
            .read()
            .expect("registry poisoned")
            .get(full)
            .cloned();
        match entry {
            Some(e) if e.kind == kind => Ok(e),
            _ => Err(PluginError::Internal(format!("入口点不存在：{full}"))),
        }
    }

    pub fn resolve_wire(&self, wire: &str) -> Option<String> {
        self.wire_to_full
            .read()
            .expect("registry poisoned")
            .get(wire)
            .cloned()
    }

    /// 入口点图标（Iconify 名，GUI 展示用）。
    pub fn entry_icon(&self, full_name: &str) -> Option<String> {
        self.handlers
            .read()
            .expect("registry poisoned")
            .get(full_name)
            .and_then(|e| e.icon.clone())
    }

    /// 模型工具列表：只含 UserAndModel 工具，名字为 wire name（Q7/Q11）。
    pub fn model_tools(&self) -> Vec<ToolSchema> {
        self.handlers
            .read()
            .expect("registry poisoned")
            .values()
            .filter(|e| e.kind == EntryKind::Tool && e.policy == CallerPolicy::UserAndModel)
            .map(|e| ToolSchema {
                name: full_to_wire(&e.full_name),
                description: e.description.clone(),
                input_schema: serde_json::to_value(&e.params).unwrap_or_default(),
            })
            .collect()
    }

    pub fn namespaces(&self) -> Vec<String> {
        self.entries
            .read()
            .expect("registry poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

pub fn tool_def(name: &str, description: &str, policy: CallerPolicy) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: description.into(),
        params: crate::kernel::contract::empty_params(),
        policy,
        timeout: None,
        icon: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::dispatch::ToolCallContext;
    use crate::kernel::logger::Logger;
    use crate::kernel::services::ServiceId;
    use serde_json::{Value, json};

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
    fn wire_collision_rejected() {
        let registry = Registry::new(ServiceHandles::default(), logger());
        let desc = PluginDescriptor {
            info: Info {
                namespace: "a".into(),
                tools: vec![
                    tool_def("b_c", "t1", CallerPolicy::UserAndModel),
                    tool_def("b", "t2", CallerPolicy::UserAndModel),
                ],
                ..Default::default()
            },
            register: |_| Ok(()),
        };
        // a::b_c → a_b_c，与 a::b 不同 wire；真正冲突需 namespace 组合。
        assert!(registry.register_plugin(desc).is_ok());
        let desc2 = PluginDescriptor {
            info: Info {
                namespace: "a_b".into(),
                tools: vec![tool_def("c", "t3", CallerPolicy::UserAndModel)],
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
        assert_eq!(registry.model_tools()[0].name, "demo_hello");
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
}
