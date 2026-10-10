//! `PostgreSQL` wire protocol constants.
//!
//! Reference: <https://www.postgresql.org/docs/current/protocol.html>

/// Protocol version 3.0 (major=3, minor=0).
///
/// Encoded as: (major << 16) | minor = (3 << 16) | 0 = 196608
pub const PROTOCOL_VERSION_3_0: i32 = 196_608;

/// SSL request code (not a protocol version).
///
/// Encoded as: (1234 << 16) | 5679 = 80877103
pub const SSL_REQUEST_CODE: i32 = 80_877_103;

/// GSSAPI encryption request code.
///
/// Encoded as: (1234 << 16) | 5680 = 80877104
pub const GSSAPI_ENC_REQUEST_CODE: i32 = 80_877_104;

/// Cancel request code (used in `CancelRequest` message).
///
/// Encoded as: (1234 << 16) | 5678 = 80877102
pub const CANCEL_REQUEST_CODE: i32 = 80_877_102;

/// Wire format code for a parameter or column value.
///
/// Postgres encodes this as an `i16` in Bind and `RowDescription`: `0` = text,
/// `1` = binary. Modeling it as an enum keeps the closed set exhaustive and
/// removes bare `== 1` comparisons at the use sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatCode {
    Text,
    Binary,
}

impl FormatCode {
    /// Decode a wire format code. Only `0` (text) and `1` (binary) are defined
    /// by the protocol; any other value is treated as text (lenient).
    #[must_use]
    pub const fn from_i16(code: i16) -> Self {
        match code {
            1 => Self::Binary,
            _ => Self::Text,
        }
    }

    /// Encode to the wire `i16` representation (`0` = text, `1` = binary).
    #[must_use]
    pub const fn as_i16(self) -> i16 {
        match self {
            Self::Text => 0,
            Self::Binary => 1,
        }
    }

    /// Returns true if this is the binary format.
    #[must_use]
    pub const fn is_binary(self) -> bool {
        matches!(self, Self::Binary)
    }
}

/// NULL value length indicator in `DataRow` and Bind messages.
pub const NULL_VALUE_LENGTH: i32 = -1;

/// Unspecified type OID (used in Parse when client wants server to infer type).
pub const UNSPECIFIED_TYPE_OID: i32 = 0;
