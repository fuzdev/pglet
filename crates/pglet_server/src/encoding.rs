//! Wire protocol encoding helpers.

use pglet_sql::executor::{QueryResult, coerce_value};
use pglet_sql::types::{ColumnDesc, TypeOid, Value, value_from_binary};
use pglet_sql::{Error, Result, Severity, plain_wire_fields};
use pglet_wire::protocol::FormatCode;
use pglet_wire::{BackendMessage, TransactionStatus};

/// Convert a Value to wire format bytes (text format).
///
/// Text rendering is single-sourced in `pglet_sql::types::value_to_text` (shared
/// with `::text` casts and array elements); this just encodes it to bytes. Bytea
/// goes out as the `\xdeadbeef` hex literal here — raw bytes are the binary path
/// only (`value_to_binary`).
pub fn value_to_bytes(v: &Value) -> Option<Vec<u8>> {
    pglet_sql::types::value_to_text(v).map(String::into_bytes)
}

/// Encode row value at column `i` as text wire bytes, honoring the column's type.
///
/// The single column-aware text path for the wire encoders (both simple and
/// extended protocol). A `timestamptz` column renders with its `+00` offset — a bare
/// `Value::Timestamp` can't carry that distinction, only the column `TypeOid` does
/// (see `value_to_text_with_oid`). Falls back to the value-only `value_to_bytes` when
/// the column is absent (defensive — for a SELECT, `columns` always aligns with the
/// row).
fn text_bytes_at(columns: &[ColumnDesc], i: usize, v: &Value) -> Option<Vec<u8>> {
    columns.get(i).map_or_else(
        || value_to_bytes(v),
        |col| pglet_sql::types::value_to_text_with_oid(v, col.type_oid).map(String::into_bytes),
    )
}

/// Decode a Bind parameter's wire bytes into a `Value`.
///
/// `oid` is the parameter's inferred type (from `infer_param_types`), `format`
/// its per-parameter Bind format code, and `param_number` its 1-based position in
/// the Bind message. The binary path delegates to `pglet_core`'s `value_from_binary`
/// (the inverse of `value_to_binary` / `encode_array_binary`), which is where
/// `param_number` is spent; the text path routes through `coerce_value` (the same
/// parsing as `::text` casts), so it needs no separate per-type code. A SQL NULL
/// parameter is handled by the caller, not here.
///
/// # Errors
///
/// Returns an error for the malformations the two paths check — on the binary path
/// whatever `value_from_binary` refuses (a read past the payload's end, `08P01`; bytes
/// the type's reader left unread, `22P03 incorrect binary data format in bind
/// parameter <param_number>`; a `jsonb` version byte that is not `1`, `XX000`; the
/// array and `numeric` readers' own refusals; and the temporal readers' range
/// verdicts, `22000`), on the text path whatever `coerce_value` refuses — plus `22021`
/// for a NUL or illegal UTF-8 in text either path carries, a text-format value being
/// validated whatever its type, as 18.6's `exec_bind_message` runs
/// `pg_client_to_server` on it before any input function. It is **not** an error for
/// every malformation 18.6 refuses: a binary array header's flags word, lower bound, and
/// an element OID other than an array type pglet recognises are not checked, so a
/// malformed header can decode, silently, to a wrong value (`EDGES.md` row 140).
pub fn decode_param(
    bytes: &[u8],
    oid: TypeOid,
    format: FormatCode,
    param_number: usize,
) -> Result<Value> {
    if format.is_binary() {
        value_from_binary(bytes, oid, param_number)
    } else {
        let text = pglet_core::encoding::text_from_utf8(bytes.to_vec())?;
        coerce_value(Value::Text(text), oid)
    }
}

/// One statement's simple-query result messages **without** the trailing
/// `ReadyForQuery`: the `RowDescription` and `DataRow`s when the statement returns a
/// result set (`ResultShape::Columns`, zero columns included), and the
/// `CommandComplete` always.
///
/// A single simple-`Query` message carrying multiple statements emits each
/// statement's full result set, then **one** `ReadyForQuery` for the whole message
/// (Postgres semantics) — so the multi-statement path accumulates these per
/// statement and appends the single `ReadyForQuery` itself.
/// [`query_result_to_messages`] appends it for the single-statement case.
pub fn query_result_to_messages_no_ready(result: &QueryResult, tag: &str) -> Vec<BackendMessage> {
    let mut messages = Vec::new();

    // The `RowDescription` + `DataRow`s go out for a statement that returns a **result
    // set**, whatever its width — a zero-column one is described `fields=0` and its
    // rows follow, exactly as 18.6 answers `SELECT *` over a relation whose last column
    // was dropped. Keying this on the column list instead is `EDGES.md` row 59: the
    // descriptor went unsent and the rows went with it, under a `SELECT 2` tag.
    if let pglet_sql::executor::ResultShape::Columns(descs) = &result.shape {
        let columns: Vec<pglet_wire::FieldDescription> = descs
            .iter()
            .map(|c| {
                pglet_wire::FieldDescription::new(
                    &c.name,
                    c.type_oid,
                    c.type_modifier,
                    c.table_oid,
                    c.column_id,
                )
            })
            .collect();
        messages.push(BackendMessage::RowDescription { columns });

        // Data rows. Encode each value column-type-aware so a `timestamptz` gets its
        // `+00` offset (the `Value` alone can't distinguish it from `timestamp`).
        for row in &result.rows {
            let values: Vec<Option<Vec<u8>>> = row
                .iter()
                .enumerate()
                .map(|(i, v)| text_bytes_at(descs, i, v))
                .collect();
            messages.push(BackendMessage::DataRow { values });
        }
    }

    // A statement's NOTICEs precede its CommandComplete (Postgres emits the
    // `IF EXISTS`-skip notice here rather than swallowing it).
    for notice in &result.notices {
        messages.push(notice_response(notice));
    }
    messages.push(BackendMessage::CommandComplete { tag: tag.into() });
    messages
}

/// Convert a query result to wire protocol messages (simple query protocol).
///
/// The trailing `ReadyForQuery` carries `status` directly, so callers never
/// patch it after the fact — pass `conn.transaction_status`.
pub fn query_result_to_messages(
    result: &QueryResult,
    tag: &str,
    status: TransactionStatus,
) -> Vec<BackendMessage> {
    let mut messages = query_result_to_messages_no_ready(result, tag);
    messages.push(BackendMessage::ReadyForQuery { status });
    messages
}

/// Convert query result to extended protocol messages (no `ReadyForQuery`).
///
/// Extended protocol: client already has `RowDescription` from Describe,
/// so Execute only sends `DataRow`* + `CommandComplete`.
///
/// # Arguments
/// * `result` - Query result with rows to encode
/// * `tag` - Command completion tag (e.g. "SELECT 1")
/// * `result_formats` - Format codes from Bind. Empty means all text.
pub fn query_result_to_extended(
    result: &QueryResult,
    tag: &str,
    result_formats: &[FormatCode],
) -> Vec<BackendMessage> {
    let mut messages = Vec::new();

    // Encode DataRows per format requested in Bind
    for row in &result.rows {
        let values: Vec<Option<Vec<u8>>> = row
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if format_code_at(result_formats, i).is_binary() {
                    result.columns().get(i).map_or_else(
                        || pglet_sql::types::value_to_binary(v),
                        |col| pglet_sql::types::value_to_binary_with_oid(v, col.type_oid),
                    )
                } else {
                    text_bytes_at(result.columns(), i, v)
                }
            })
            .collect();
        messages.push(BackendMessage::DataRow { values });
    }

    for notice in &result.notices {
        messages.push(notice_response(notice));
    }
    messages.push(BackendMessage::CommandComplete { tag: tag.into() });
    messages
}

/// A `NoticeResponse` carrying a `NOTICE`-severity, `00000`
/// (`successful_completion`) message — the SQLSTATE Postgres's `DROP … IF EXISTS`
/// skip notice uses (an `ereport(NOTICE, …)` with no explicit errcode).
///
/// Built through the same `pglet_core::plain_wire_fields` every other send site uses,
/// so the severity pair (`S` **and** the unlocalized `V`) cannot be right on one path
/// and missing on another — which is exactly what it was: three hand-written `[SCM]`
/// triples, none of them sending `V`.
fn notice_response(message: &str) -> BackendMessage {
    BackendMessage::NoticeResponse {
        fields: plain_wire_fields(Severity::Notice, "00000", message),
    }
}

/// Resolve a per-position wire format code from a Bind/Execute format list.
///
/// Postgres protocol format-code rules (shared by Bind parameters and Execute
/// result columns):
/// - empty list = all text,
/// - single element = applies to every position,
/// - otherwise per-position (padded with text if shorter than the position).
pub fn format_code_at(formats: &[FormatCode], idx: usize) -> FormatCode {
    match formats.len() {
        0 => FormatCode::Text,                                      // Empty = all text
        1 => formats[0], // Single = every position the same
        _ => formats.get(idx).copied().unwrap_or(FormatCode::Text), // Per-position
    }
}

/// Create an error response from `pglet_sql::Error` with proper SQLSTATE.
///
/// The trailing `ReadyForQuery` carries `status` directly — pass the actual
/// `conn.transaction_status` (the caller has already failed the transaction
/// when appropriate) so it never needs patching afterward.
pub fn error_response(err: &Error, status: TransactionStatus) -> Vec<BackendMessage> {
    let mut msgs = extended_error(err);
    msgs.push(BackendMessage::ReadyForQuery { status });
    msgs
}

/// Error response for extended protocol (no `ReadyForQuery`).
pub fn extended_error(err: &Error) -> Vec<BackendMessage> {
    vec![BackendMessage::ErrorResponse {
        fields: err.wire_fields(Severity::Error),
    }]
}

/// A **FATAL** `ErrorResponse` — severity `FATAL`, which tells the client the backend
/// is closing the connection rather than failing one statement.
///
/// Its members are startup refusals: a packet asking for a `client_encoding` pglet cannot
/// serve, a value another setting's decider refuses (`quote_all_identifiers` on), and a
/// `lock_timeout` / `default_transaction_isolation` its type refuses. 18.6 answers an
/// unservable encoding the same way (`FATAL 22023 invalid value for parameter
/// "client_encoding": "…"`, measured with a raw v3 client, connection closed); the
/// alternative here is not a statement error but silent corruption of every string on the
/// connection, so the severity is the honest one.
pub fn fatal_with_code(code: &str, message: &str) -> Vec<BackendMessage> {
    vec![BackendMessage::ErrorResponse {
        fields: plain_wire_fields(Severity::Fatal, code, message),
    }]
}

/// A bare `ErrorResponse` (no `ReadyForQuery`) carrying an explicit SQLSTATE and
/// message — for the wire-level codes that don't originate from a `pglet_sql::Error`.
/// The caller appends `ReadyForQuery` when the simple protocol needs it.
///
/// That set has shrunk to the **frame**-level ones: the COPY sub-protocol's `57014` /
/// `08P01` and `Bind`'s parameter-count `08P01`. A missing prepared statement / portal
/// (`26000` / `34000`) and the aborted-block guard (`25P02`) each have an `Error`
/// variant now and route through [`extended_error`], because each is one message and
/// one SQLSTATE reached from several sites. The three that remain are violations of
/// the wire framing with no engine meaning, so giving them `pglet_core` variants would
/// put protocol concerns in the leaf crate — the wrong direction.
pub fn error_with_code(code: &str, message: &str) -> Vec<BackendMessage> {
    vec![BackendMessage::ErrorResponse {
        fields: plain_wire_fields(Severity::Error, code, message),
    }]
}

#[cfg(test)]
mod tests {
    use super::{
        error_with_code, extended_error, fatal_with_code, notice_response, value_to_bytes,
    };
    use pglet_sql::types::Value;
    use pglet_wire::BackendMessage;

    /// The field codes of a message, in send order — the shape a client actually reads.
    fn codes(message: &BackendMessage) -> String {
        let fields = match message {
            BackendMessage::ErrorResponse { fields }
            | BackendMessage::NoticeResponse { fields } => fields,
            other => panic!("not an error/notice message: {other:?}"),
        };
        fields.iter().map(|(c, _)| *c as char).collect()
    }

    fn value_of(message: &BackendMessage, code: u8) -> Option<String> {
        let fields = match message {
            BackendMessage::ErrorResponse { fields }
            | BackendMessage::NoticeResponse { fields } => fields,
            other => panic!("not an error/notice message: {other:?}"),
        };
        fields
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, v)| v.clone())
    }

    // One error per send site. All four used to hand-roll their own `[SCM]` triple and
    // none of them sent `V`; these pin that every site now goes through the one builder.

    #[test]
    fn an_engine_error_carries_its_structured_fields() {
        let err = pglet_sql::Error::NotNullViolation {
            table: "t".into(),
            column: "c".into(),
            row: Some(Box::new(pglet_sql::FailingRow::new(vec![
                None,
                Some("a".into()),
            ]))),
        };
        let messages = extended_error(&err);
        assert_eq!(codes(&messages[0]), "SVCMDstc");
        assert_eq!(value_of(&messages[0], b'V').as_deref(), Some("ERROR"));
        assert_eq!(
            value_of(&messages[0], b'D').as_deref(),
            Some("Failing row contains (null, a).")
        );
    }

    #[test]
    fn a_raw_code_error_carries_the_severity_pair() {
        let messages = error_with_code("08P01", "bind message supplies 0 parameters");
        assert_eq!(codes(&messages[0]), "SVCM");
        assert_eq!(value_of(&messages[0], b'S').as_deref(), Some("ERROR"));
        assert_eq!(value_of(&messages[0], b'V').as_deref(), Some("ERROR"));
    }

    #[test]
    fn a_fatal_carries_the_severity_pair() {
        let messages = fatal_with_code("22023", "invalid value for parameter");
        assert_eq!(codes(&messages[0]), "SVCM");
        assert_eq!(value_of(&messages[0], b'S').as_deref(), Some("FATAL"));
        assert_eq!(value_of(&messages[0], b'V').as_deref(), Some("FATAL"));
    }

    #[test]
    fn a_notice_carries_the_severity_pair() {
        let message = notice_response("table \"x\" does not exist, skipping");
        assert_eq!(codes(&message), "SVCM");
        assert_eq!(value_of(&message, b'S').as_deref(), Some("NOTICE"));
        assert_eq!(value_of(&message, b'V').as_deref(), Some("NOTICE"));
        assert_eq!(value_of(&message, b'C').as_deref(), Some("00000"));
    }

    // A text-format `bytea` parameter is read by the type's input function with no lexer in
    // front of it, and a binary one is its bytes: 18.6 stores four bytes for the text `abcd`
    // in either format, and reads `\x6162` as hex only in the text one.
    #[test]
    fn a_text_format_bytea_parameter_is_read_by_the_input_function() {
        use super::decode_param;
        use pglet_sql::types::{TypeOid, Value};
        use pglet_wire::protocol::FormatCode;

        let read = |bytes: &[u8], format| decode_param(bytes, TypeOid::Bytea, format, 1);
        for (text, bytes) in [
            (&b"abcd"[..], &b"abcd"[..]),
            (b" ", b" "),
            (b"\\141\\\\", b"a\\"),
            (b"\\x6162 63", b"abc"),
        ] {
            assert_eq!(
                read(text, FormatCode::Text).unwrap(),
                Value::Bytea(bytes.to_vec())
            );
        }
        assert_eq!(
            read(b"\\X61", FormatCode::Text).unwrap_err().sqlstate(),
            "22P02"
        );
        assert_eq!(
            read(b"\\x6162", FormatCode::Binary).unwrap(),
            Value::Bytea(b"\\x6162".to_vec())
        );
    }

    // `decode_param` is where a bound `$N`'s bytes enter. A text-format value is validated
    // in the server encoding whatever its type -- 18.6's `exec_bind_message` runs
    // `pg_client_to_server` before any input function, measured for `text`, `int4`,
    // `bytea` and `jsonb` -- and a binary one per type, through `value_from_binary`.
    #[test]
    fn a_bound_parameter_is_validated_in_the_server_encoding_in_both_formats() {
        use super::decode_param;
        use pglet_sql::types::TypeOid;
        use pglet_wire::protocol::FormatCode;

        for oid in [TypeOid::Text, TypeOid::Int4, TypeOid::Bytea, TypeOid::Jsonb] {
            let err = decode_param(b"1\0", oid, FormatCode::Text, 1).unwrap_err();
            assert_eq!(err.sqlstate(), "22021", "{oid:?}: {err}");
            assert_eq!(
                err.to_string(),
                "invalid byte sequence for encoding \"UTF8\": 0x00",
                "{oid:?}"
            );
        }
        for format in [FormatCode::Text, FormatCode::Binary] {
            let err = decode_param(b"x\xe9y", TypeOid::Text, format, 1).unwrap_err();
            assert_eq!(
                err.to_string(),
                "invalid byte sequence for encoding \"UTF8\": 0xe9 0x79",
                "{format:?}"
            );
            // The accept side: a valid multi-byte character binds.
            assert_eq!(
                decode_param("x\u{e9}y".as_bytes(), TypeOid::Text, format, 1).unwrap(),
                Value::Text("x\u{e9}y".into()),
                "{format:?}"
            );
        }
    }

    // `value_to_bytes` is the text-format wire encoder (simple-query results and
    // any text-format column in the extended protocol). The differential fixture
    // harness reads results in binary (tokio-postgres `query()`), so it can't reach
    // this path — these cases pin it directly.

    #[test]
    fn bytea_text_format_is_pg_hex_literal() {
        // Postgres renders bytea in text format as `\xdeadbeef`, not raw bytes.
        assert_eq!(
            value_to_bytes(&Value::Bytea(vec![0xde, 0xad, 0xbe, 0xef])),
            Some(b"\\xdeadbeef".to_vec())
        );
        // Low nibbles and NUL bytes keep their leading zeros.
        assert_eq!(
            value_to_bytes(&Value::Bytea(vec![0x00, 0xff, 0x10])),
            Some(b"\\x00ff10".to_vec())
        );
        // Empty bytea is the bare prefix `\x`.
        assert_eq!(value_to_bytes(&Value::Bytea(vec![])), Some(b"\\x".to_vec()));
    }

    #[test]
    fn uuid_text_format_is_canonical_hyphenated() {
        let bytes = [
            0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44,
            0x00, 0x00,
        ];
        assert_eq!(
            value_to_bytes(&Value::Uuid(bytes)),
            Some(b"550e8400-e29b-41d4-a716-446655440000".to_vec())
        );
    }
}
