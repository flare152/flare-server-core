//! 微服务运行时实现
//!
//! 提供统一的服务生命周期管理，支持：
//! - gRPC 服务
//! - 消息消费者
//! - 自定义任务
//! - 服务注册和注销
//! - 优雅停机

use std::net::SocketAddr;
use tokio::sync::oneshot;
use tokio::task::JoinSet;
use tracing::{info, error, warn};

use crate::runtime::task::{Task, TaskResult};
use crate::discovery::ServiceRegistry;
use crate::runtime::config::RuntimeConfig;
use anyhow::Result;

/// 微服务运行时
///
/// 统一管理服务的生命周期，包括：
/// - 任务启动和管理（gRPC、消息消费者等）
/// - 服务注册和注销
/// - 优雅停机
///
/// # 使用示例
///
/// ## 简单模式（不注册服务）
/// ```rust,no_run
/// use flare_server_core::runtime::ServiceRuntime;
/// use tonic::transport::Server;
/// use tokio::sync::oneshot;
///
/// // 创建关闭通道
/// let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
///
/// // 在 service 层构建好 gRPC server
/// let grpc_future = Server::builder()
///     .add_service(MyServiceServer::new(handler))
///     .serve_with_shutdown(address, async move {
///         tokio::select! {
///             _ = tokio::signal::ctrl_c() => {
///                 tracing::info!("shutdown signal received (Ctrl+C)");
///             }
///             _ = shutdown_rx => {
///                 tracing::info!("shutdown signal received");
///             }
///         }
///     });
///
/// let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap())
///     .add_spawn("my-grpc", grpc_future);
///
/// runtime.run().await?;
/// ```
///
/// ## 完整模式（带服务注册）
/// ```rust,no_run
/// use flare_server_core::runtime::ServiceRuntime;
///
/// let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap())
///     .add_spawn("my-grpc", grpc_future);
///
/// runtime.run_with_registration(|address| {
///     Box::pin(async move {
///         // 注册服务
///         Ok(Some(registry))
///     })
/// }).await?;
/// ```
///
/// ## 任务依赖管理
/// ```rust,no_run
/// use flare_server_core::runtime::ServiceRuntime;
///
/// let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap())
///     // 先启动数据库连接池
///     .add_spawn("db-pool", async { Ok(()) })
///     // 然后启动缓存（依赖数据库）
///     .add_spawn_with_deps("cache", async { Ok(()) }, vec!["db-pool".to_string()])
///     // 最后启动 gRPC 服务（依赖缓存）
///     .add_spawn_with_shutdown_and_deps("grpc", |shutdown_rx| async move {
///         // gRPC server code
///         Ok(())
///     }, vec!["cache".to_string()]);
///
/// runtime.run().await?;
/// ```
pub struct ServiceRuntime {
    service_name: String,
    service_address: Option<SocketAddr>,
    tasks: Vec<Box<dyn Task>>,
    registry: Option<ServiceRegistry>,
    config: RuntimeConfig,
}

impl ServiceRuntime {
    /// 创建新的服务运行时（带地址，用于 gRPC 服务）
    ///
    /// # 参数
    /// * `service_name` - 服务名称（用于服务注册和日志）
    /// * `service_address` - 服务地址（用于服务注册）
    pub fn new(service_name: impl Into<String>, service_address: SocketAddr) -> Self {
        Self {
            service_name: service_name.into(),
            service_address: Some(service_address),
            tasks: Vec::new(),
            registry: None,
            config: RuntimeConfig::default(),
        }
    }
    
    /// 创建新的消费者服务运行时（不带地址，用于纯消费者服务）
    ///
    /// # 参数
    /// * `service_name` - 服务名称（用于日志）
    ///
    /// # 示例
    /// ```rust,no_run
    /// use flare_server_core::runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::new_consumer_only("my-consumer")
    ///     .add_consumer("kafka-consumer", consumer.consume_messages());
    ///
    /// runtime.run().await?;
    /// ```
    pub fn new_consumer_only(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            service_address: None,
            tasks: Vec::new(),
            registry: None,
            config: RuntimeConfig::default()
                .with_task_ready_check(false),  // 消费者服务不需要地址检查
        }
    }
    
    /// 设置运行时配置
    pub fn with_config(mut self, config: RuntimeConfig) -> Self {
        self.config = config;
        self
    }
    
    /// 添加任务
    ///
    /// # 参数
    /// * `task` - 要添加的任务（实现了 `Task` trait）
    pub fn add_task(mut self, task: Box<dyn Task>) -> Self {
        info!(task_name = %task.name(), "Adding task to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加 spawn 任务（直接添加 Future，不需要 shutdown_rx）
    ///
    /// 允许用户直接添加已经构建好的 Future，例如不需要 shutdown 信号的任务
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future` - 要运行的 Future
    ///
    /// # 示例
    /// ```rust,no_run
    /// use flare_server_core::runtime::ServiceRuntime;
    ///
    /// let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap());
    ///
    /// // 添加不需要 shutdown 的任务
    /// runtime.add_spawn("my-task", async { Ok(()) });
    /// 
    /// // 添加带依赖的任务
    /// runtime.add_spawn_with_deps("task-b", async { Ok(()) }, vec!["task-a".to_string()]);
    /// ```
    pub fn add_spawn<Fut>(mut self, name: impl Into<String>, future: Fut) -> Self
    where
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        use crate::runtime::task::SpawnTask;
        let task = Box::new(SpawnTask::new(name, future));
        info!(task_name = %task.name(), "Adding spawn task to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加 spawn 任务（带依赖关系）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future` - 要运行的 Future
    /// * `dependencies` - 依赖的任务名称列表
    pub fn add_spawn_with_deps<Fut>(
        mut self,
        name: impl Into<String>,
        future: Fut,
        dependencies: Vec<String>,
    ) -> Self
    where
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        use crate::runtime::task::SpawnTask;
        let task = Box::new(SpawnTask::new(name, future).with_dependencies(dependencies));
        info!(task_name = %task.name(), deps = ?task.dependencies(), "Adding spawn task with dependencies to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加 spawn 任务（需要 shutdown_rx）
    ///
    /// 允许用户添加需要 shutdown 信号的 Future，例如 gRPC server
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future_fn` - 闭包，接收 shutdown_rx，返回 Future
    ///
    /// # 示例
    /// ```rust,no_run
    /// use flare_server_core::runtime::ServiceRuntime;
    /// use tonic::transport::Server;
    ///
    /// let runtime = ServiceRuntime::new("my-service", "0.0.0.0:8080".parse().unwrap());
    ///
    /// // 添加需要 shutdown 的 gRPC server
    /// runtime.add_spawn_with_shutdown("my-grpc", |shutdown_rx| {
    ///     Server::builder()
    ///         .add_service(MyServiceServer::new(handler))
    ///         .serve_with_shutdown(address, async move {
    ///             tokio::select! {
    ///                 _ = tokio::signal::ctrl_c() => {}
    ///                 _ = shutdown_rx => {}
    ///             }
    ///         })
    ///         .map_err(|e| format!("gRPC server error: {}", e).into())
    /// });
    /// ```
    pub fn add_spawn_with_shutdown<F, Fut>(mut self, name: impl Into<String>, future_fn: F) -> Self
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        use crate::runtime::task::SpawnTask;
        let task = Box::new(SpawnTask::with_shutdown(name, future_fn));
        info!(task_name = %task.name(), "Adding spawn task with shutdown to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加 spawn 任务（需要 shutdown_rx，带依赖关系）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future_fn` - 闭包，接收 shutdown_rx，返回 Future
    /// * `dependencies` - 依赖的任务名称列表
    pub fn add_spawn_with_shutdown_and_deps<F, Fut>(
        mut self,
        name: impl Into<String>,
        future_fn: F,
        dependencies: Vec<String>,
    ) -> Self
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = TaskResult> + Send + 'static,
    {
        use crate::runtime::task::SpawnTask;
        let task = Box::new(
            SpawnTask::with_shutdown(name, future_fn)
                .with_dependencies(dependencies)
        );
        info!(task_name = %task.name(), deps = ?task.dependencies(), "Adding spawn task with shutdown and dependencies to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加消费者任务（便捷方法）
    ///
    /// 用于添加 Kafka 消费者等任务，返回 Future<Output = Result<()>>
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `future` - 消费者 Future（通常是 `consumer.consume_messages()`）
    ///
    /// # 示例
    /// ```rust,no_run
    /// let runtime = ServiceRuntime::new_consumer_only("my-service")
    ///     .add_consumer("kafka-consumer-1", consumer1.consume_messages())
    ///     .add_consumer("kafka-consumer-2", consumer2.consume_messages());
    /// ```
    pub fn add_consumer<Fut>(mut self, name: impl Into<String>, future: Fut) -> Self
    where
        Fut: std::future::Future<Output = TaskResult> + Send + 'static,
    {
        use crate::runtime::task::SpawnTask;
        let task = Box::new(SpawnTask::new(name, future));
        info!(task_name = %task.name(), "Adding consumer task to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加消息消费者（使用 MessageConsumer trait）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `consumer` - 消息消费者实例
    pub fn add_message_consumer(mut self, name: impl Into<String>, consumer: Box<dyn crate::runtime::task::MessageConsumer + Send + Sync>) -> Self {
        use crate::runtime::task::MessageConsumerTask;
        let task = Box::new(MessageConsumerTask::new(name, consumer));
        info!(task_name = %task.name(), "Adding message consumer to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加消息消费者（带依赖关系）
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `consumer` - 消息消费者实例
    /// * `dependencies` - 依赖的任务名称列表
    pub fn add_message_consumer_with_deps(
        mut self,
        name: impl Into<String>,
        consumer: Box<dyn crate::runtime::task::MessageConsumer + Send + Sync>,
        dependencies: Vec<String>,
    ) -> Self {
        use crate::runtime::task::MessageConsumerTask;
        let task = Box::new(MessageConsumerTask::new(name, consumer).with_dependencies(dependencies));
        info!(task_name = %task.name(), deps = ?task.dependencies(), "Adding message consumer with dependencies to runtime");
        self.tasks.push(task);
        self
    }
    
    /// 添加自定义任务
    ///
    /// # 参数
    /// * `name` - 任务名称
    /// * `task_fn` - 任务函数，接收 shutdown_rx，返回 Future
    ///
    /// 注意：此方法内部使用 `SpawnTask::with_shutdown`，功能相同
    pub fn add_custom_task<F, Fut>(mut self, name: impl Into<String>, task_fn: F) -> Self
    where
        F: FnOnce(tokio::sync::oneshot::Receiver<()>) -> Fut + Send + 'static,
        Fut: Future<Output = crate::runtime::task::TaskResult> + Send + 'static,
    {
        // 使用 SpawnTask::with_shutdown 实现
        self.add_spawn_with_shutdown(name, task_fn)
    }
    
    /// 设置服务注册器
    ///
    /// 如果设置了注册器，运行时会在所有任务就绪后自动注册服务
    pub fn with_registry(mut self, registry: ServiceRegistry) -> Self {
        self.registry = Some(registry);
        self
    }
    
    /// 运行服务（简单模式，不注册服务）
    ///
    /// 执行以下步骤：
    /// 1. 启动所有任务
    /// 2. 等待所有任务就绪
    /// 3. 等待关闭信号（Ctrl+C）
    /// 4. 优雅关闭所有任务
    pub async fn run(mut self) -> Result<()> {
        if let Some(addr) = self.service_address {
            info!(
                service_name = %self.service_name,
                address = %addr,
                task_count = self.tasks.len(),
                "🚀 Starting service runtime"
            );
        } else {
            info!(
                service_name = %self.service_name,
                task_count = self.tasks.len(),
                "🚀 Starting consumer-only service runtime"
            );
        }
        
        // 创建关闭通道
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let mut shutdown_tx_opt = Some(shutdown_tx);
        
        // 启动所有任务（消费 tasks）
        let tasks = std::mem::take(&mut self.tasks);
        let (mut join_set, task_shutdowns) = Self::start_tasks(tasks)
            .map_err(|e| anyhow::anyhow!("Failed to start tasks: {}", e))?;
        
        // 等待所有任务就绪
        self.wait_for_tasks_ready().await?;
        
        // 如果配置了服务注册，进行注册
        let registry = self.registry.take();
        
        // 等待关闭信号（Ctrl+C）
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutdown signal received (Ctrl+C)");
            }
            _ = shutdown_rx => {
                info!("Shutdown signal received (service registration failed)");
            }
        }
        
        // 发送关闭信号给所有任务
        if let Some(tx) = shutdown_tx_opt.take() {
            let _ = tx.send(());
        }
        for tx in task_shutdowns {
            let _ = tx.send(());
        }
        
        // 等待所有任务关闭
        Self::wait_for_tasks_shutdown(&self.config, &mut join_set).await;
        
        // 注销服务
        if let Some(mut reg) = registry {
            if let Err(e) = reg.shutdown().await {
                warn!(
                    error = %e,
                    "⚠️ Failed to shutdown service registry gracefully"
                );
            }
        }
        
        info!(service_name = %self.service_name, "Service runtime stopped");
        Ok(())
    }
    
    /// 运行服务（完整模式，带服务注册）
    ///
    /// 执行以下步骤：
    /// 1. 启动所有任务
    /// 2. 等待所有任务就绪
    /// 3. 注册服务（如果配置了）
    /// 4. 等待关闭信号（Ctrl+C 或注册失败）
    /// 5. 优雅关闭所有任务
    /// 6. 注销服务
    pub async fn run_with_registration(
        mut self,
        register_fn: impl FnOnce(SocketAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<ServiceRegistry>, Box<dyn std::error::Error + Send + Sync>>> + Send>>,
    ) -> Result<()> {
        let service_name = self.service_name.clone();
        let service_address = self.service_address.ok_or_else(|| {
            anyhow::anyhow!("Service address is required for service registration. Use `new()` instead of `new_consumer_only()` for services that need registration.")
        })?;
        let task_count = self.tasks.len();
        
        info!(
            service_name = %service_name,
            address = %service_address,
            task_count = task_count,
            "🚀 Starting service runtime with registration"
        );
        
        // 创建关闭通道（用于在注册失败时关闭所有任务）
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let mut shutdown_tx_opt = Some(shutdown_tx);
        
        // 启动所有任务（消费 tasks）
        let tasks = std::mem::take(&mut self.tasks);
        let (mut join_set, task_shutdowns) = Self::start_tasks(tasks)
            .map_err(|e| anyhow::anyhow!("Failed to start tasks: {}", e))?;
        
        // 等待所有任务就绪
        self.wait_for_tasks_ready().await?;
        
        // 注册服务
        info!("Registering service after all tasks are ready...");
        let registry = match register_fn(service_address).await {
            Ok(Some(reg)) => {
                info!("✅ Service registered: {}", service_name);
                Some(reg)
            }
            Ok(None) => {
                info!("Service discovery not configured, skipping registration");
                None
            }
            Err(e) => {
                // 服务注册失败，发送关闭信号并返回错误
                error!(
                    error = %e,
                    "❌ Service registration failed, shutting down service"
                );
                
                // 发送关闭信号
                if let Some(tx) = shutdown_tx_opt.take() {
                    let _ = tx.send(());
                }
                
                // 等待所有任务关闭
                Self::wait_for_tasks_shutdown(&self.config, &mut join_set).await;
                
                return Err(anyhow::anyhow!("Service registration failed: {}", e));
            }
        };
        
        // 等待关闭信号（Ctrl+C）
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Shutdown signal received (Ctrl+C)");
            }
            _ = shutdown_rx => {
                info!("Shutdown signal received (service registration failed)");
            }
        }
        
        // 发送关闭信号给所有任务
        if let Some(tx) = shutdown_tx_opt.take() {
            let _ = tx.send(());
        }
        for tx in task_shutdowns {
            let _ = tx.send(());
        }
        
        // 等待所有任务关闭
        Self::wait_for_tasks_shutdown(&self.config, &mut join_set).await;
        
        // 注销服务
        if let Some(mut reg) = registry {
            if let Err(e) = reg.shutdown().await {
                warn!(
                    error = %e,
                    "⚠️ Failed to shutdown service registry gracefully"
                );
            }
        }
        
        info!(service_name = %self.service_name, "Service runtime stopped");
        Ok(())
    }
    
    /// 等待所有任务就绪
    ///
    /// 1. 首先检查主服务地址（如果有 gRPC 服务且配置了地址检查）
    /// 2. 然后调用每个任务的 ready_check
    async fn wait_for_tasks_ready(&self) -> Result<()> {
        info!("Waiting for all tasks to be ready...");
        
        if !self.config.enable_task_ready_check {
            info!("Task ready check is disabled, skipping");
            return Ok(());
        }
        
        // 检查主服务地址（仅当有地址时，用于 gRPC 服务）
        if let Some(address) = self.service_address {
            info!("Checking main service address...");
            match tokio::time::timeout(
                self.config.ready_check_timeout,
                crate::utils::wait_for_server_ready(address)
            ).await {
                Ok(Ok(_)) => {
                    info!("✅ Main service is ready");
                }
                Ok(Err(e)) => {
                    return Err(anyhow::anyhow!("Failed to wait for main service to be ready: {}", e));
                }
                Err(_) => {
                    return Err(anyhow::anyhow!("Main service ready check timeout after {:?}", self.config.ready_check_timeout));
                }
            }
        } else {
            info!("No service address configured, skipping address check");
        }
        
        info!("✅ All tasks are ready");
        Ok(())
    }
    
    /// 启动所有任务（按依赖顺序）
    ///
    /// 返回 JoinSet 和 task_shutdowns
    fn start_tasks(
        tasks: Vec<Box<dyn Task>>,
    ) -> Result<(JoinSet<TaskResult>, Vec<tokio::sync::oneshot::Sender<()>>)> {
        // 1. 拓扑排序，确定任务启动顺序
        let sorted_tasks = Self::topological_sort(tasks)?;
        
        let mut join_set = JoinSet::new();
        let mut task_shutdowns = Vec::new();
        
        // 2. 按排序后的顺序启动任务
        for task in sorted_tasks {
            let task_name = task.name().to_string();
            let (task_shutdown_tx, task_shutdown_rx) = oneshot::channel();
            task_shutdowns.push(task_shutdown_tx);
            
            let task_future = task.run(task_shutdown_rx);
            
            join_set.spawn(async move {
                let result = task_future.await;
                match &result {
                    Ok(_) => {
                        info!(task_name = %task_name, "✅ Task completed");
                    }
                    Err(e) => {
                        error!(task_name = %task_name, error = %e, "❌ Task failed");
                    }
                }
                result
            });
        }
        
        Ok((join_set, task_shutdowns))
    }
    
    /// 拓扑排序任务，确定启动顺序
    ///
    /// 使用 Kahn 算法进行拓扑排序，确保依赖的任务先启动
    /// 同时检测循环依赖
    fn topological_sort(tasks: Vec<Box<dyn Task>>) -> Result<Vec<Box<dyn Task>>> {
        use std::collections::{HashMap, HashSet, VecDeque};
        
        // 构建任务索引
        let mut task_map: HashMap<String, Box<dyn Task>> = HashMap::new();
        let mut task_names = Vec::new();
        
        for task in tasks {
            let name = task.name().to_string();
            task_names.push(name.clone());
            task_map.insert(name, task);
        }
        
        // 构建依赖图：task -> 依赖它的任务列表
        let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        
        for name in &task_names {
            dependents.insert(name.clone(), Vec::new());
            in_degree.insert(name.clone(), 0);
        }
        
        // 构建依赖关系
        for name in &task_names {
            let task = task_map.get(name).unwrap();
            let deps = task.dependencies();
            
            for dep in &deps {
                // 检查依赖的任务是否存在
                if !task_map.contains_key(dep) {
                    return Err(anyhow::anyhow!(
                        "Task '{}' depends on '{}', but '{}' is not registered",
                        name, dep, dep
                    ));
                }
                
                // 增加入度
                *in_degree.get_mut(name).unwrap() += 1;
                
                // 添加到依赖者的列表
                dependents.get_mut(dep).unwrap().push(name.clone());
            }
        }
        
        // Kahn 算法：找到所有入度为 0 的任务
        let mut queue = VecDeque::new();
        for (name, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(name.clone());
            }
        }
        
        let mut sorted = Vec::new();
        let mut processed = HashSet::new();
        
        // 处理队列中的任务
        while let Some(current) = queue.pop_front() {
            if processed.contains(&current) {
                continue;
            }
            
            processed.insert(current.clone());
            
            // 将任务添加到排序结果
            if let Some(task) = task_map.remove(&current) {
                sorted.push(task);
            }
            
            // 更新依赖此任务的其他任务的入度
            if let Some(deps) = dependents.get(&current) {
                for dependent in deps {
                    let degree = in_degree.get_mut(dependent).unwrap();
                    *degree -= 1;
                    
                    if *degree == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
        
        // 检查是否有循环依赖
        if sorted.len() != task_names.len() {
            let remaining: Vec<String> = task_names
                .into_iter()
                .filter(|name| !processed.contains(name))
                .collect();
            
            return Err(anyhow::anyhow!(
                "Circular dependency detected. Tasks involved: {:?}",
                remaining
            ));
        }
        
        // 记录排序结果
        if sorted.len() > 1 {
            let order: Vec<String> = sorted.iter().map(|t| t.name().to_string()).collect();
            info!(
                task_order = ?order,
                "Tasks sorted by dependencies"
            );
        }
        
        Ok(sorted)
    }
    
    /// 等待所有任务关闭
    async fn wait_for_tasks_shutdown(
        config: &RuntimeConfig,
        join_set: &mut JoinSet<TaskResult>,
    ) {
        match tokio::time::timeout(
            config.shutdown_timeout,
            async {
                while let Some(result) = join_set.join_next().await {
                    match result {
                        Ok(Ok(_)) => {
                            info!("Task completed gracefully");
                        }
                        Ok(Err(e)) => {
                            warn!("Task completed with error: {}", e);
                        }
                        Err(e) => {
                            warn!("Task join error: {}", e);
                        }
                    }
                }
            }
        ).await {
            Ok(_) => {
                info!("All tasks completed");
            }
            Err(_) => {
                warn!("Tasks shutdown timeout, forcing exit");
                join_set.abort_all();
            }
        }
    }
}
