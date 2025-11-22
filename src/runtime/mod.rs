//! 微服务运行时框架
//!
//! 提供统一的服务生命周期管理，支持多种类型的任务（gRPC、消息消费者等）
//!
//! # 设计理念
//!
//! 1. **插件化任务系统**：通过 `Task` trait 支持不同类型的任务
//! 2. **统一生命周期管理**：启动、就绪检查、服务注册、优雅关闭
//! 3. **并发任务管理**：使用 `JoinSet` 管理所有后台任务
//! 4. **优雅停机**：确保所有任务正确关闭，服务正确注销
//!
//! # 使用示例
//!
//! ## 简单模式（不注册服务）
//! ```rust,no_run
//! use flare_server_core::runtime::ServiceRuntime;
//! use tonic::transport::Server;
//!
//! let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap())
//!     .add_grpc_server(
//!         "my-grpc",
//!         "0.0.0.0:50051".parse().unwrap(),
//!         |builder| builder.add_service(MyServiceServer::new(handler))
//!     );
//!
//! runtime.run().await?;
//! ```
//!
//! ## 完整模式（带服务注册）
//! ```rust,no_run
//! use flare_server_core::runtime::ServiceRuntime;
//!
//! let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap())
//!     .add_grpc_server(...);
//!
//! runtime.run_with_registration(|address| {
//!     Box::pin(async move {
//!         // 注册服务
//!         Ok(Some(registry))
//!     })
//! }).await?;
//! ```

pub mod task;
pub mod runtime;
pub mod config;

pub use task::{Task, TaskResult, MessageConsumer, MessageConsumerTask, SpawnTask};
pub use runtime::ServiceRuntime;
pub use config::RuntimeConfig;
