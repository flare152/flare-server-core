//! Flare Server Core 错误处理模块
//!
//! 提供统一的错误处理机制，支持国际化、错误代码分类和错误转换
//! 与 flare-core 的错误定义完全适配

pub mod builder;
pub mod code;
pub mod conversions;
pub mod flare_error;
pub mod grpc;
pub mod localized;
pub mod proto;

// 重新导出公共类型和函数
pub use builder::ErrorBuilder;
pub use code::{ErrorCategory, ErrorCode};
pub use flare_error::{FlareError, Result, ServerError};
pub use grpc::{GrpcError, GrpcErrorExt, GrpcResult};
pub use localized::LocalizedError;
pub use proto::{
    from_rpc_status, localized_to_rpc_status, map_error_code_to_proto, map_proto_code_to_error,
    ok_status, to_localized, to_rpc_status,
};

// 兼容性导出
pub use flare_error::FlareError as FlareServerError;

/// 基础设施层默认使用的结果类型
pub type InfraResult<T> = anyhow::Result<T>;

/// 将基础设施错误转换为 `FlareError`
pub fn map_infra_error<E, S>(error: E, code: ErrorCode, message: S) -> FlareError
where
    E: std::fmt::Display,
    S: Into<String>,
{
    ErrorBuilder::new(code, message.into())
        .details(error.to_string())
        .build_error()
}

/// `InfraResult` 的辅助扩展，用于快速转换为统一的业务错误类型
pub trait InfraResultExt<T> {
    fn into_flare<S>(self, code: ErrorCode, message: S) -> Result<T>
    where
        S: Into<String>;
}

impl<T> InfraResultExt<T> for InfraResult<T> {
    fn into_flare<S>(self, code: ErrorCode, message: S) -> Result<T>
    where
        S: Into<String>,
    {
        self.map_err(|err| map_infra_error(err, code, message))
    }
}
