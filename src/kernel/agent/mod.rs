//! Agent 核心调度层（kernel/agent）：agent loop、工具调度、会话调度与 RPC。
//!
//! - `loop_mod`：agent loop（LLM 唯一决策者，串行工具执行，护栏/压缩/中断消费）；
//! - `dispatch`：统一工具执行（CallerPolicy 双墙、懒注册、schema 校验、超时/取消、审计）；
//! - `session`：Session scheduler（SessionKey/Goal/切换决策/交接摘要，独立内核级模块，
//!   不占 ServiceId；`session::switch` 工具入口见 `crate::kernel::plugin::session`）；
//! - `rpc`：帧类型与 Kernel 组装（standalone 进程内内核，GUI 唯一通信面）；
//! - `balance` / `cache`：余额查询与上下文缓存命中率统计（RPC 辅助能力）。

pub mod balance;
pub mod cache;
pub mod dispatch;
pub mod loop_mod;
pub mod rpc;
pub mod session;
