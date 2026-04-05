//! gRPC 工具函数模块
//!
//! 提供基础的 gRPC 工具函数,不依赖其他模块

mod context_utils;
mod metadata_codec;

pub use metadata_codec::{decode_context_from_metadata, encode_context_to_metadata};

pub use context_utils::{
    create_traced_metadata, error_to_status, extract_ctx_from_request_opt, extract_device_id,
    extract_session_id, extract_trace_id, extract_user_id, require_ctx_from_request,
    require_request_id, require_tenant_id, require_user_id, wait_for_server_ready,
};
