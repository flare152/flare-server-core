//! 停机信号模块
//!
//! 提供多种停机信号源和信号合并器

mod channel;
mod composite;
mod ctrl_c;
mod r#trait;
mod unix;

pub use channel::ChannelSignal;
pub use composite::CompositeSignal;
pub use ctrl_c::CtrlCSignal;
pub use r#trait::ShutdownSignal;
pub use unix::{UnixSignal, UnixSignalKind};
