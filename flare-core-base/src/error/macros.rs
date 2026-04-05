//! 错误处理宏
//!
//! 提供便捷的宏用于快速创建 FlareError

/// 快速构造 `FlareError`（仅 code + reason）。
///
/// # 示例
///
/// ```rust
/// use flare_core_base::error::{flare_err, ErrorCode};
///
/// let err = flare_err!(ErrorCode::BadRequest, "Invalid input");
/// ```
#[macro_export]
macro_rules! flare_err {
    ($code:expr, $reason:expr) => {{ $crate::error::ErrorBuilder::new($code, $reason).build_error() }};
}

/// 构造带 `details` 的 `FlareError`。
///
/// # 示例
///
/// ```rust
/// use flare_core_base::error::{flare_err_details, ErrorCode};
///
/// let err = flare_err_details!(ErrorCode::BadRequest, "reason", "details");
/// ```
#[macro_export]
macro_rules! flare_err_details {
    ($code:expr, $reason:expr, $details:expr) => {{
        $crate::error::ErrorBuilder::new($code, $reason)
            .details($details)
            .build_error()
    }};
}

/// 构造带 `details` + 多个 `param(k, v)` 的 `FlareError`。
///
/// # 示例
///
/// ```rust
/// use flare_core_base::error::{flare_err_params, ErrorCode};
///
/// let err = flare_err_params!(ErrorCode::BadRequest, "reason", "details", "key1" => "value1", "key2" => "value2");
/// ```
#[macro_export]
macro_rules! flare_err_params {
    ($code:expr, $reason:expr, $details:expr, $( $k:expr => $v:expr ),+ $(,)?) => {{
        let mut b = $crate::error::ErrorBuilder::new($code, $reason).details($details);
        $(
            b = b.param($k, $v);
        )+
        b.build_error()
    }};
}
