//! Model gateway layer (§5).
//!
//! One profile source, strict cloud/local separation, OpenAI-compatible tool
//! calls, usage accounting, error classification, four-layer context compaction
//! and cancellation.
pub mod context;
pub mod gateway;
pub mod openai_compatible;
pub mod profile;
pub mod transport;
pub mod usage;

pub use context::compact_messages;
pub use gateway::{CancelToken, ModelError, ModelGateway, ModelRequest};
pub use openai_compatible::OpenAiCompatibleGateway;
pub use profile::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile};
pub use transport::{CancelInitiator, TransportError, TransportEvent, TransportRequest};
pub use usage::{ToolSchema, UsageDelta};

#[cfg(test)]
mod tests;
