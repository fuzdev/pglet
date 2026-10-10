//! The PostgreSQL wire protocol (v3) as a pure codec: bytes to message structs and back, no
//! I/O. The native server owns the sockets and the session state.

mod codec;
mod messages;
pub mod protocol;
mod session;

pub use codec::{decode_message, encode_message};
pub use messages::{
    BackendMessage, DescribeKind, FieldDescription, FrontendMessage, TransactionStatus,
};
pub use session::{Session, SessionState};

use thiserror::Error;

/// Errors that can occur during wire protocol encoding/decoding.
#[derive(Debug, Error)]
pub enum WireError {
    /// Need more bytes to complete message parsing.
    #[error("incomplete message: need {0} more bytes")]
    Incomplete(usize),

    /// Unknown or unsupported message type byte.
    #[error("invalid message type: {0}")]
    InvalidMessageType(u8),

    /// Message structure doesn't match expected format.
    #[error("invalid message format: {0}")]
    InvalidFormat(String),

    /// General protocol violation.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Integer value too large for protocol field.
    #[error("value overflow: {0}")]
    Overflow(&'static str),
}

pub type Result<T> = std::result::Result<T, WireError>;
