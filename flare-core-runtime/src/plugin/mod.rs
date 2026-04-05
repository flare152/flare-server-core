//! 插件系统模块
//!
//! 提供插件抽象和管理

mod manager;
mod r#trait;

pub use manager::PluginManager;
pub use r#trait::{Plugin, PluginContext};
