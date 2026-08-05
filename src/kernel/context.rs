//! 插件注册上下文：两段式契约第二阶段（注入句柄 + 绑定 handler）。

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

use crate::kernel::agent::dispatch::{CommandHandler, EventHandler, ToolHandler};
use crate::kernel::contract::{CallerPolicy, Info, PluginError, full_name, full_to_wire};
use crate::kernel::logger::LoggerHandle;
use crate::kernel::plugin::services::ServiceHandles;
use crate::kernel::registry::{EntryKind, Handler, RegisteredEntry};

/// 注册目标：kernel 注册表内部结构，只经 EntryRegistrar 暴露受限写入。
pub struct RegistrarTargets<'a> {
    pub handlers: &'a RwLock<HashMap<String, RegisteredEntry>>,
    pub wire_to_full: &'a RwLock<HashMap<String, String>>,
}

/// 只允许登记 info 中声明过的短名（声明与实现一致，Q4）。
pub struct EntryRegistrar<'a> {
    namespace: &'a str,
    declared: &'a Info,
    targets: RegistrarTargets<'a>,
}

impl<'a> EntryRegistrar<'a> {
    pub fn new(namespace: &'a str, declared: &'a Info, targets: RegistrarTargets<'a>) -> Self {
        Self {
            namespace,
            declared,
            targets,
        }
    }

    pub fn tool(&self, short: &str, handler: ToolHandler) -> Result<(), PluginError> {
        let def = self
            .declared
            .tools
            .iter()
            .find(|t| t.name == short)
            .ok_or_else(|| PluginError::UndeclaredEntry(short.into()))?;
        let full = full_name(self.namespace, short);
        let entry = RegisteredEntry {
            full_name: full.clone(),
            kind: EntryKind::Tool,
            policy: def.policy,
            timeout: def.timeout.map(Duration::from_secs),
            description: def.description.clone(),
            icon: def.icon.clone(),
            params: def.params.clone(),
            handler: Handler::Tool(handler),
        };
        self.insert(full, entry)
    }

    pub fn command(&self, short: &str, handler: CommandHandler) -> Result<(), PluginError> {
        let def = self
            .declared
            .commands
            .iter()
            .find(|c| c.name == short)
            .ok_or_else(|| PluginError::UndeclaredEntry(short.into()))?;
        let full = full_name(self.namespace, short);
        let entry = RegisteredEntry {
            full_name: full.clone(),
            kind: EntryKind::Command,
            // 命令恒为 UserOnly（Q3）：结构上不给 policy 字段。
            policy: CallerPolicy::UserOnly,
            timeout: None,
            description: def.description.clone(),
            icon: def.icon.clone(),
            params: def.params.clone(),
            handler: Handler::Command(handler),
        };
        self.insert(full, entry)
    }

    pub fn event(&self, name: &str, handler: EventHandler) -> Result<(), PluginError> {
        if !self.declared.events.iter().any(|e| e.name == name) {
            return Err(PluginError::UndeclaredEntry(name.into()));
        }
        let full = full_name(self.namespace, name);
        let entry = RegisteredEntry {
            full_name: full.clone(),
            kind: EntryKind::Event,
            policy: CallerPolicy::UserOnly,
            timeout: None,
            description: String::new(),
            icon: None,
            params: crate::kernel::contract::empty_params(),
            handler: Handler::Event(handler),
        };
        self.insert(full, entry)
    }

    fn insert(&self, full: String, entry: RegisteredEntry) -> Result<(), PluginError> {
        let mut handlers = self.targets.handlers.write().expect("registry poisoned");
        if handlers.contains_key(&full) {
            return Err(PluginError::DuplicateEntry(full));
        }
        handlers.insert(full.clone(), entry);
        self.targets
            .wire_to_full
            .write()
            .expect("registry poisoned")
            .insert(full_to_wire(&full), full);
        Ok(())
    }
}

/// 插件注册上下文（ADR-0003 两段式第二阶段）。
pub struct PluginContext<'a> {
    pub handles: ServiceHandles,
    pub logger: LoggerHandle,
    pub registrar: EntryRegistrar<'a>,
}

/// 内核插件注册上下文（ADR-0035）：与 PluginContext 同形，但注入**全量**服务句柄——
/// 内核插件在信任边界内，不按 requires 过滤；requires 对内核插件无意义。
pub struct KernelContext<'a> {
    pub handles: ServiceHandles,
    pub logger: LoggerHandle,
    pub registrar: EntryRegistrar<'a>,
}
