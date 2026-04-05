//! KV存储抽象模块
//!
//! 提供统一的KV存储接口，支持多种后端（etcd、consul等）

pub mod backend;
pub mod store;

pub use backend::{KvBackend, KvEntry, KvError};
pub use store::KvStore;
