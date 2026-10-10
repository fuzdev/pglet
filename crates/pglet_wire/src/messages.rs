use pglet_core::TypeOid;
// `ReadyForQuery` carries the session's transaction status, but the state machine
// behind it is the engine's, not the codec's — the embed facade and the server both
// hold one, so the type lives in `pglet_core` and this crate names it rather than
// defining a second copy of the same three states.
pub use pglet_core::TransactionStatus;

use crate::protocol::FormatCode;

/// Messages sent by the frontend (client)
#[derive(Debug, Clone)]
pub enum FrontendMessage {
    /// SSL negotiation request (client wants to upgrade to TLS).
    SslRequest,

    /// GSSAPI encryption negotiation request (8-byte special packet, the sibling of
    /// [`Self::SslRequest`]). A libpq client built with GSSAPI and `gssencmode=prefer`
    /// (a common default) sends this *first*, before the startup message. pglet has
    /// no GSSAPI support, so the server declines it (replies a single `N`) and stays
    /// in startup — matching a Postgres backend built without GSSAPI.
    GssEncRequest,

    /// Cancel request (16-byte special packet): the client asks the backend to cancel
    /// the in-progress query identified by `pid`/`secret` (psql Ctrl-C, a driver
    /// query-timeout). It is fire-and-forget — the protocol expects **no reply**, the
    /// server just drops the connection. pglet implements no query cancellation: the
    /// server closes this packet's connection with no reply, rather than mis-parsing it
    /// as a startup message and spuriously answering `AuthenticationOk`, and a
    /// statement running on the backend the request names runs to completion.
    CancelRequest { pid: i32, secret: i32 },

    /// Initial startup message (no type byte).
    Startup {
        version: i32,
        params: Vec<(String, String)>,
    },

    /// Simple query
    Query { sql: String },

    /// Parse (extended query protocol)
    Parse {
        name: String,
        query: String,
        /// Parameter type OIDs specified by client.
        /// None = type unspecified (0 in protocol) or unknown type.
        param_types: Vec<Option<TypeOid>>,
    },

    /// Bind (extended query protocol)
    Bind {
        portal: String,
        statement: String,
        /// Parameter format codes. Empty means all text.
        param_formats: Vec<FormatCode>,
        params: Vec<Option<Vec<u8>>>,
        /// Result format codes. Empty means all text.
        result_formats: Vec<FormatCode>,
    },

    /// Execute (extended query protocol)
    Execute { portal: String, max_rows: i32 },

    /// Describe
    Describe { kind: DescribeKind, name: String },

    /// Close (extended query protocol)
    Close { kind: DescribeKind, name: String },

    /// Sync
    Sync,

    /// Flush (extended query protocol - requests immediate send of buffered output)
    Flush,

    /// `CopyData` (`d`) — one chunk of a `COPY … FROM STDIN` byte stream. The
    /// payload is the raw (text-format) COPY bytes; the server accumulates them
    /// until `CopyDone`.
    CopyData(Vec<u8>),

    /// `CopyDone` (`c`) — the client has finished sending `COPY … FROM STDIN`
    /// data; the server ingests the accumulated buffer.
    CopyDone,

    /// `CopyFail` (`f`) — the client aborts an in-progress `COPY … FROM STDIN`.
    /// The string is the client's failure message.
    CopyFail(String),

    /// Terminate
    Terminate,
}

#[derive(Debug, Clone, Copy)]
pub enum DescribeKind {
    Statement,
    Portal,
}

impl DescribeKind {
    /// Decode the target-kind byte used by Describe and Close messages
    /// (`S` = prepared statement, `P` = portal). Returns `None` for any other byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            b'S' => Some(Self::Statement),
            b'P' => Some(Self::Portal),
            _ => None,
        }
    }
}

/// Messages sent by the backend (server)
#[derive(Debug, Clone)]
pub enum BackendMessage {
    /// Authentication request
    AuthenticationOk,

    /// Parameter status (server config)
    ParameterStatus { name: String, value: String },

    /// Backend key data (for cancellation)
    /// TODO(protocol-3.2): In protocol 3.2+, `secret_key` is variable-length Byten (4-256 bytes).
    /// Currently using fixed i32 (4 bytes) for protocol 3.0 compatibility.
    BackendKeyData { process_id: i32, secret_key: i32 },

    /// Ready for query
    ReadyForQuery { status: TransactionStatus },

    /// Row description (column metadata)
    RowDescription { columns: Vec<FieldDescription> },

    /// Data row
    DataRow { values: Vec<Option<Vec<u8>>> },

    /// Command complete
    CommandComplete { tag: String },

    /// Empty query response
    EmptyQueryResponse,

    /// Error response
    ErrorResponse { fields: Vec<(u8, String)> },

    /// Notice response
    NoticeResponse { fields: Vec<(u8, String)> },

    /// Parse complete
    ParseComplete,

    /// Bind complete
    BindComplete,

    /// Close complete
    CloseComplete,

    /// No data
    NoData,

    /// Parameter description (extended query protocol)
    ParameterDescription { param_types: Vec<i32> },

    /// `CopyInResponse` (`G`) — the server is ready to receive a `COPY … FROM
    /// STDIN` data stream. `overall_format` is `0` for the text format; for text,
    /// every `column_formats` entry is `0` (one per transferred column).
    CopyInResponse {
        overall_format: i8,
        column_formats: Vec<i16>,
    },

    /// `CopyOutResponse` (`H`) — the server is about to send a `COPY … TO STDOUT`
    /// data stream. Same format-code shape as `CopyInResponse`.
    CopyOutResponse {
        overall_format: i8,
        column_formats: Vec<i16>,
    },

    /// `CopyData` (`d`) — one chunk of a `COPY … TO STDOUT` byte stream (the
    /// server emits one per row, each including its trailing newline).
    CopyData(Vec<u8>),

    /// `CopyDone` (`c`) — the server has finished sending `COPY … TO STDOUT`
    /// data; a `CommandComplete` follows.
    CopyDone,
}

/// One field of a `RowDescription` message (a Postgres wire `T` row-field
/// descriptor).
///
/// Named for the wire role to avoid colliding with `pglet_core::ColumnDesc`, the
/// SQL-layer column descriptor it's built from.
#[derive(Debug, Clone)]
pub struct FieldDescription {
    pub name: String,
    pub table_oid: i32,
    pub column_id: i16,
    pub type_oid: i32,
    pub type_size: i16,
    pub type_modifier: i32,
    pub format: FormatCode,
}

impl FieldDescription {
    /// Build a row-field descriptor from a result column's name, type, and wire
    /// metadata. `type_modifier` is the Postgres `atttypmod` encoding (`-1` for a
    /// type with no modifier); `table_oid` / `column_id` are the source
    /// base-table OID + 1-based attnum (both `0` for a computed/expression column).
    /// `type_size` derives from the type. Describe replies are always text format.
    pub fn new(
        name: impl Into<String>,
        type_oid: TypeOid,
        type_modifier: i32,
        table_oid: i32,
        column_id: i16,
    ) -> Self {
        Self {
            name: name.into(),
            table_oid,
            column_id,
            type_oid: type_oid as i32,
            type_size: type_oid.wire_size(),
            type_modifier,
            format: FormatCode::Text, // Always text in Describe responses
        }
    }
}
