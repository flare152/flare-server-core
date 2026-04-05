//! 任务抽象模块
//!
//! 提供统一的 Task 抽象，所有运行时管理的任务都必须实现此 trait

mod manager;
mod spawn;
mod state;
mod r#trait;

pub use manager::TaskManager;
pub use spawn::SpawnTask;
pub use state::TaskState;
pub use r#trait::{Task, TaskResult};
