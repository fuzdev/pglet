use crate::messages::{BackendMessage, DescribeKind, FrontendMessage, TransactionStatus};
use crate::protocol::{CANCEL_REQUEST_CODE, FormatCode, GSSAPI_ENC_REQUEST_CODE, SSL_REQUEST_CODE};
use crate::{Result, WireError};

/// Decodes a frontend message from bytes.
///
/// Returns the message and number of bytes consumed.
///
/// # Errors
///
/// Returns `WireError::Incomplete` if more bytes are needed, or other errors
/// for malformed messages.
pub fn decode_message(buf: &[u8], is_startup: bool) -> Result<(FrontendMessage, usize)> {
    if is_startup {
        decode_startup(buf)
    } else {
        decode_regular(buf)
    }
}

fn decode_startup(buf: &[u8]) -> Result<(FrontendMessage, usize)> {
    if buf.len() < 8 {
        return Err(WireError::Incomplete(8 - buf.len()));
    }

    let len = read_i32_as_usize(&buf[0..4])?;
    // A startup message is at least 8 bytes: the 4-byte length plus the 4-byte
    // version/request code. A smaller declared length is malformed and must error
    // here, before the params loop below where `len - 1` would otherwise underflow
    // (wrapping to `usize::MAX` in release, then over-reading) and panic.
    if len < 8 {
        return Err(WireError::InvalidFormat(format!(
            "startup length {len} below minimum of 8"
        )));
    }
    if buf.len() < len {
        return Err(WireError::Incomplete(len - buf.len()));
    }

    let code = i32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);

    // Check for SSL request (special 8-byte message)
    if len == 8 && code == SSL_REQUEST_CODE {
        return Ok((FrontendMessage::SslRequest, 8));
    }

    // GSSAPI encryption request (special 8-byte message, the sibling of SSLRequest).
    // A libpq client with `gssencmode=prefer`/`require` (the default when libpq is
    // built with GSSAPI) sends this *before* the startup message; without this branch
    // it falls through to the regular-startup path below and the server spuriously
    // answers `AuthenticationOk`, desyncing the handshake.
    if len == 8 && code == GSSAPI_ENC_REQUEST_CODE {
        return Ok((FrontendMessage::GssEncRequest, 8));
    }

    // Cancel request (special 16-byte message): the 4-byte code is followed by the
    // target backend's `pid` (bytes 8..12) and `secret` (bytes 12..16). The
    // `buf.len() < len` guard above already ensured all 16 bytes are present, so this
    // indexing cannot panic.
    if len == 16 && code == CANCEL_REQUEST_CODE {
        let pid = i32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let secret = i32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
        return Ok((FrontendMessage::CancelRequest { pid, secret }, 16));
    }

    // Regular startup message - code is the protocol version
    let mut params = Vec::new();
    let mut pos = 8;
    // `pos + 1 < len` rather than `pos < len - 1`: the guard above keeps `len >= 8`,
    // but the additive form is underflow-proof regardless and reads the same.
    //
    // A missing NUL terminator below yields a zero-length key, which breaks the
    // loop (treated as end-of-params). This is a deliberately lenient parse of the
    // pre-auth startup handshake with no query-result impact — it does not fail
    // loud the way `Reader::read_cstring` does for post-auth messages.
    while pos + 1 < len {
        let key_end = buf[pos..].iter().position(|&b| b == 0).unwrap_or(0);
        let key = String::from_utf8_lossy(&buf[pos..pos + key_end]).to_string();
        pos += key_end + 1;

        if key.is_empty() {
            break;
        }

        let val_end = buf[pos..].iter().position(|&b| b == 0).unwrap_or(0);
        let value = String::from_utf8_lossy(&buf[pos..pos + val_end]).to_string();
        pos += val_end + 1;

        params.push((key, value));
    }

    Ok((
        FrontendMessage::Startup {
            version: code,
            params,
        },
        len,
    ))
}

fn decode_regular(buf: &[u8]) -> Result<(FrontendMessage, usize)> {
    if buf.len() < 5 {
        return Err(WireError::Incomplete(5 - buf.len()));
    }

    let msg_type = buf[0];
    let len = read_i32_as_usize(&buf[1..5])?;
    // The length field counts its own 4 bytes, so a well-formed message declares
    // `len >= 4` (an empty body). A smaller value is malformed and must error here:
    // otherwise `total_len < 5` and the `&buf[5..total_len]` body slice below
    // panics (start > end), which under `panic = "abort"` aborts the whole server
    // on one crafted packet.
    if len < 4 {
        return Err(WireError::InvalidFormat(format!(
            "message length {len} below minimum of 4"
        )));
    }
    let total_len = len + 1; // type byte + length includes itself

    if buf.len() < total_len {
        return Err(WireError::Incomplete(total_len - buf.len()));
    }

    // Every field below is read through `Reader`, which bounds-checks against the
    // (already length-validated) body and returns a `WireError` rather than
    // panicking — the bytes are untrusted socket input and `panic = "abort"`
    // would turn one crafted packet into a whole-process abort.
    let body = &buf[5..total_len];
    let mut r = Reader::new(body);

    let msg = match msg_type {
        b'Q' => {
            let sql = r.read_cstring()?;
            FrontendMessage::Query { sql }
        }
        b'P' => {
            let name = r.read_cstring()?;
            let query = r.read_cstring()?;
            let param_count = r.read_i16_as_usize()?;
            let mut param_types = Vec::with_capacity(param_count);
            for _ in 0..param_count {
                // Convert OID to TypeOid (0 or unknown types are stored as None)
                param_types.push(pglet_core::TypeOid::from_oid(r.read_i32()?));
            }
            FrontendMessage::Parse {
                name,
                query,
                param_types,
            }
        }
        b'B' => {
            let portal = r.read_cstring()?;
            let statement = r.read_cstring()?;

            // Parse parameter format codes
            let format_count = r.read_i16_as_usize()?;
            let mut param_formats = Vec::with_capacity(format_count);
            for _ in 0..format_count {
                param_formats.push(FormatCode::from_i16(r.read_i16()?));
            }

            // Parse parameter values
            let param_count = r.read_i16_as_usize()?;
            let mut params = Vec::with_capacity(param_count);
            for _ in 0..param_count {
                let value_len = r.read_i32()?;
                if value_len == crate::protocol::NULL_VALUE_LENGTH {
                    params.push(None); // NULL
                } else {
                    let len = usize::try_from(value_len).map_err(|_| {
                        WireError::InvalidFormat(format!("negative parameter length: {value_len}"))
                    })?;
                    params.push(Some(r.read_bytes(len)?.to_vec()));
                }
            }

            // Parse result format codes
            let result_format_count = r.read_i16_as_usize()?;
            let mut result_formats = Vec::with_capacity(result_format_count);
            for _ in 0..result_format_count {
                result_formats.push(FormatCode::from_i16(r.read_i16()?));
            }

            FrontendMessage::Bind {
                portal,
                statement,
                param_formats,
                params,
                result_formats,
            }
        }
        b'E' => {
            let portal = r.read_cstring()?;
            let max_rows = r.read_i32()?;
            FrontendMessage::Execute { portal, max_rows }
        }
        b'C' => {
            let kind = DescribeKind::from_byte(r.read_u8()?)
                .ok_or_else(|| WireError::InvalidFormat("invalid close kind".into()))?;
            let name = r.read_cstring()?;
            FrontendMessage::Close { kind, name }
        }
        b'D' => {
            let kind = DescribeKind::from_byte(r.read_u8()?)
                .ok_or_else(|| WireError::InvalidFormat("invalid describe kind".into()))?;
            let name = r.read_cstring()?;
            FrontendMessage::Describe { kind, name }
        }
        b'S' => FrontendMessage::Sync,
        b'H' => FrontendMessage::Flush,
        // CopyData: the remaining body bytes are the COPY payload chunk. `remaining`
        // is bounds-clamped, so a zero-length (or any) body can't panic.
        b'd' => FrontendMessage::CopyData(r.remaining().to_vec()),
        b'c' => FrontendMessage::CopyDone,
        // CopyFail carries a NUL-terminated failure message; a missing terminator is
        // a graceful error (the length-checked `read_cstring`), not a panic.
        b'f' => FrontendMessage::CopyFail(r.read_cstring()?),
        b'X' => FrontendMessage::Terminate,
        _ => return Err(WireError::InvalidMessageType(msg_type)),
    };

    Ok((msg, total_len))
}

/// A length-checked cursor over a message body.
///
/// Every read advances the cursor and returns a [`WireError`] (never panics)
/// when fewer bytes remain than the field needs — the codec's contract, since
/// the body is untrusted socket input. Counts read from the body bound how many
/// times we *attempt* a read, but each individual read still re-checks the
/// remaining length, so a bogus count or length surfaces as an error, not an
/// out-of-bounds index or an unbounded allocation (value bytes are only
/// allocated after `read_bytes` confirms they exist).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// The not-yet-consumed tail.
    fn remaining(&self) -> &'a [u8] {
        // `pos` never exceeds `buf.len()` (every advance goes through a checked
        // read), but clamp defensively so this can't panic.
        &self.buf[self.pos.min(self.buf.len())..]
    }

    /// Take `n` bytes, erroring if fewer remain.
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| WireError::InvalidFormat("length overflow".into()))?;
        let slice = self.buf.get(self.pos..end).ok_or_else(|| {
            WireError::InvalidFormat(format!(
                "truncated message: need {n} bytes, {} remain",
                self.buf.len().saturating_sub(self.pos)
            ))
        })?;
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }

    fn read_i16(&mut self) -> Result<i16> {
        let b = self.read_bytes(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    fn read_i32(&mut self) -> Result<i32> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read an `i16` count as a `usize`, erroring on a negative count.
    fn read_i16_as_usize(&mut self) -> Result<usize> {
        let val = self.read_i16()?;
        usize::try_from(val).map_err(|_| WireError::InvalidFormat(format!("negative count: {val}")))
    }

    /// Read a NUL-terminated string, erroring if no terminator remains.
    fn read_cstring(&mut self) -> Result<String> {
        let rest = self.remaining();
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| WireError::InvalidFormat("unterminated string".into()))?;
        let s = String::from_utf8_lossy(&rest[..end]).to_string();
        self.pos += end + 1; // consume the string and its NUL
        Ok(s)
    }
}

/// Read a big-endian `i32` length prefix as a `usize`.
///
/// Only called on the fixed 4-byte header slices, which the callers have already
/// length-checked (`buf.len() >= 5` / `>= 8`), so the direct indexing cannot
/// panic; a negative length still errors.
fn read_i32_as_usize(buf: &[u8]) -> Result<usize> {
    let val = i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    usize::try_from(val).map_err(|_| WireError::InvalidFormat(format!("negative length: {val}")))
}

fn usize_to_i32(val: usize, context: &'static str) -> Result<i32> {
    i32::try_from(val).map_err(|_| WireError::Overflow(context))
}

fn usize_to_i16(val: usize, context: &'static str) -> Result<i16> {
    i16::try_from(val).map_err(|_| WireError::Overflow(context))
}

/// Prefix `body` with its backend-message frame: the 1-byte `tag`, then the i32
/// big-endian length (`body.len() + 4` — the Postgres convention where the length
/// field counts itself). The shared tail of every length-framed backend message.
fn frame_body(tag: u8, body: &[u8], context: &'static str) -> Result<Vec<u8>> {
    let len = usize_to_i32(body.len() + 4, context)?;
    let mut buf = Vec::with_capacity(5 + body.len());
    buf.push(tag);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(body);
    Ok(buf)
}

/// Encode the body shared by `ErrorResponse`/`NoticeResponse`: each `(field_code,
/// text)` as the code byte + the text bytes + a NUL, then a final NUL terminating
/// the field list.
fn encode_response_fields(fields: &[(u8, String)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (code, msg) in fields {
        body.push(*code);
        body.extend_from_slice(msg.as_bytes());
        body.push(0);
    }
    body.push(0); // field-list terminator
    body
}

/// Encodes a backend message to bytes.
///
/// # Errors
///
/// Returns `WireError::Overflow` if any field value exceeds protocol limits.
pub fn encode_message(msg: &BackendMessage) -> Result<Vec<u8>> {
    match msg {
        BackendMessage::AuthenticationOk => {
            let mut buf = vec![b'R'];
            buf.extend_from_slice(&8i32.to_be_bytes()); // length
            buf.extend_from_slice(&0i32.to_be_bytes()); // auth ok
            Ok(buf)
        }

        BackendMessage::ParameterStatus { name, value } => {
            let mut buf = vec![b'S'];
            let len = usize_to_i32(
                4 + name.len() + 1 + value.len() + 1,
                "parameter status length",
            )?;
            buf.extend_from_slice(&len.to_be_bytes());
            buf.extend_from_slice(name.as_bytes());
            buf.push(0);
            buf.extend_from_slice(value.as_bytes());
            buf.push(0);
            Ok(buf)
        }

        BackendMessage::BackendKeyData {
            process_id,
            secret_key,
        } => {
            let mut buf = vec![b'K'];
            buf.extend_from_slice(&12i32.to_be_bytes());
            buf.extend_from_slice(&process_id.to_be_bytes());
            buf.extend_from_slice(&secret_key.to_be_bytes());
            Ok(buf)
        }

        BackendMessage::ReadyForQuery { status } => {
            let status_byte = match status {
                TransactionStatus::Idle => b'I',
                TransactionStatus::InTransaction => b'T',
                TransactionStatus::Failed => b'E',
            };
            Ok(vec![b'Z', 0, 0, 0, 5, status_byte])
        }

        BackendMessage::RowDescription { columns } => {
            let mut body = Vec::new();
            let col_count = usize_to_i16(columns.len(), "column count")?;
            body.extend_from_slice(&col_count.to_be_bytes());
            for col in columns {
                body.extend_from_slice(col.name.as_bytes());
                body.push(0);
                body.extend_from_slice(&col.table_oid.to_be_bytes());
                body.extend_from_slice(&col.column_id.to_be_bytes());
                body.extend_from_slice(&col.type_oid.to_be_bytes());
                body.extend_from_slice(&col.type_size.to_be_bytes());
                body.extend_from_slice(&col.type_modifier.to_be_bytes());
                body.extend_from_slice(&col.format.as_i16().to_be_bytes());
            }

            frame_body(b'T', &body, "row description length")
        }

        BackendMessage::DataRow { values } => {
            let mut body = Vec::new();
            let val_count = usize_to_i16(values.len(), "value count")?;
            body.extend_from_slice(&val_count.to_be_bytes());
            for val in values {
                match val {
                    Some(data) => {
                        let data_len = usize_to_i32(data.len(), "data length")?;
                        body.extend_from_slice(&data_len.to_be_bytes());
                        body.extend_from_slice(data);
                    }
                    None => {
                        body.extend_from_slice(&crate::protocol::NULL_VALUE_LENGTH.to_be_bytes());
                    }
                }
            }

            frame_body(b'D', &body, "data row length")
        }

        BackendMessage::CommandComplete { tag } => {
            let mut body = tag.clone().into_bytes();
            body.push(0);
            frame_body(b'C', &body, "command complete length")
        }

        BackendMessage::EmptyQueryResponse => Ok(vec![b'I', 0, 0, 0, 4]),

        BackendMessage::ErrorResponse { fields } => frame_body(
            b'E',
            &encode_response_fields(fields),
            "error response length",
        ),

        BackendMessage::NoticeResponse { fields } => frame_body(
            b'N',
            &encode_response_fields(fields),
            "notice response length",
        ),

        BackendMessage::ParseComplete => Ok(vec![b'1', 0, 0, 0, 4]),

        BackendMessage::BindComplete => Ok(vec![b'2', 0, 0, 0, 4]),

        BackendMessage::CloseComplete => Ok(vec![b'3', 0, 0, 0, 4]),

        BackendMessage::NoData => Ok(vec![b'n', 0, 0, 0, 4]),

        BackendMessage::ParameterDescription { param_types } => {
            let mut body = Vec::new();
            let param_count = usize_to_i16(param_types.len(), "parameter count")?;
            body.extend_from_slice(&param_count.to_be_bytes());
            for oid in param_types {
                body.extend_from_slice(&oid.to_be_bytes());
            }

            frame_body(b't', &body, "parameter description length")
        }

        // CopyInResponse (`G`) / CopyOutResponse (`H`) share a body:
        // `[overall_format: i8][num_cols: i16][per-col format: i16]*`.
        BackendMessage::CopyInResponse {
            overall_format,
            column_formats,
        } => encode_copy_response(b'G', *overall_format, column_formats),

        BackendMessage::CopyOutResponse {
            overall_format,
            column_formats,
        } => encode_copy_response(b'H', *overall_format, column_formats),

        BackendMessage::CopyData(data) => frame_body(b'd', data, "copy data length"),

        BackendMessage::CopyDone => Ok(vec![b'c', 0, 0, 0, 4]),
    }
}

/// Encode a `CopyInResponse`/`CopyOutResponse` body under the given frame byte:
/// `[overall_format: i8][num_cols: i16][per-col format: i16]*`.
fn encode_copy_response(frame: u8, overall_format: i8, column_formats: &[i16]) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(3 + column_formats.len() * 2);
    body.extend_from_slice(&overall_format.to_be_bytes());
    let col_count = usize_to_i16(column_formats.len(), "copy column count")?;
    body.extend_from_slice(&col_count.to_be_bytes());
    for fmt in column_formats {
        body.extend_from_slice(&fmt.to_be_bytes());
    }

    frame_body(frame, &body, "copy response length")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_auth_ok() {
        let msg = BackendMessage::AuthenticationOk;
        let encoded = encode_message(&msg).unwrap();
        assert_eq!(encoded, vec![b'R', 0, 0, 0, 8, 0, 0, 0, 0]);
    }

    #[test]
    fn test_encode_ready_for_query() {
        let msg = BackendMessage::ReadyForQuery {
            status: TransactionStatus::Idle,
        };
        let encoded = encode_message(&msg).unwrap();
        assert_eq!(encoded, vec![b'Z', 0, 0, 0, 5, b'I']);
    }

    /// Wrap a body in a regular frame: `[type][i32 len = body+4][body]`.
    fn frame(msg_type: u8, body: &[u8]) -> Vec<u8> {
        let len = i32::try_from(body.len() + 4).unwrap();
        let mut v = vec![msg_type];
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn valid_parse_decodes() {
        // name="", query="SELECT 1", 0 param types.
        let mut body = vec![0]; // empty name + NUL
        body.extend_from_slice(b"SELECT 1\0");
        body.extend_from_slice(&0i16.to_be_bytes()); // param count
        let (msg, consumed) = decode_message(&frame(b'P', &body), false).unwrap();
        assert_eq!(consumed, body.len() + 5);
        match msg {
            FrontendMessage::Parse {
                name,
                query,
                param_types,
            } => {
                assert_eq!(name, "");
                assert_eq!(query, "SELECT 1");
                assert!(param_types.is_empty());
            }
            other => panic!("expected Parse, got {other:?}"),
        }
    }

    #[test]
    fn valid_bind_with_one_param_decodes() {
        let mut body = vec![0]; // portal "" + NUL
        body.push(0); // statement "" + NUL
        body.extend_from_slice(&0i16.to_be_bytes()); // 0 param formats
        body.extend_from_slice(&1i16.to_be_bytes()); // 1 param
        body.extend_from_slice(&3i32.to_be_bytes()); // value len 3
        body.extend_from_slice(b"abc");
        body.extend_from_slice(&0i16.to_be_bytes()); // 0 result formats
        let (msg, _) = decode_message(&frame(b'B', &body), false).unwrap();
        match msg {
            FrontendMessage::Bind { params, .. } => {
                assert_eq!(params, vec![Some(b"abc".to_vec())]);
            }
            other => panic!("expected Bind, got {other:?}"),
        }
    }

    // Every case below feeds a *crafted* message whose declared length matches the
    // buffer (so it passes the outer length check) but whose body is internally
    // truncated/oversized. Each must return a `WireError`, never panic — under
    // release `panic = "abort"` a slice panic here would abort the whole server.

    #[test]
    fn parse_with_oversized_param_count_errors() {
        // Claims 1000 param-type OIDs but supplies none.
        let mut body = vec![0, 0]; // name NUL + query NUL
        body.extend_from_slice(&1000i16.to_be_bytes());
        assert!(decode_message(&frame(b'P', &body), false).is_err());
    }

    #[test]
    fn bind_with_oversized_param_count_errors() {
        let mut body = vec![0, 0]; // portal NUL + statement NUL
        body.extend_from_slice(&0i16.to_be_bytes()); // 0 formats
        body.extend_from_slice(&5i16.to_be_bytes()); // 5 params, none present
        assert!(decode_message(&frame(b'B', &body), false).is_err());
    }

    #[test]
    fn bind_with_param_len_past_end_errors() {
        let mut body = vec![0, 0]; // portal NUL + statement NUL
        body.extend_from_slice(&0i16.to_be_bytes()); // 0 formats
        body.extend_from_slice(&1i16.to_be_bytes()); // 1 param
        body.extend_from_slice(&1000i32.to_be_bytes()); // value len 1000
        body.extend_from_slice(b"ab"); // but only 2 bytes
        assert!(decode_message(&frame(b'B', &body), false).is_err());
    }

    #[test]
    fn query_without_terminator_errors() {
        assert!(decode_message(&frame(b'Q', b"SELECT 1"), false).is_err());
    }

    #[test]
    fn execute_truncated_max_rows_errors() {
        let mut body = b"portal\0".to_vec();
        body.extend_from_slice(&[0, 0]); // only 2 of the 4 max_rows bytes
        assert!(decode_message(&frame(b'E', &body), false).is_err());
    }

    #[test]
    fn describe_with_empty_body_errors() {
        assert!(decode_message(&frame(b'D', b""), false).is_err());
    }

    #[test]
    fn short_header_is_incomplete_not_panic() {
        assert!(matches!(
            decode_message(&[b'P', 0, 0], false),
            Err(WireError::Incomplete(_))
        ));
    }

    #[test]
    fn truncated_startup_is_incomplete_not_panic() {
        // Declares length 100 but only 8 bytes present.
        let buf = [0, 0, 0, 100, 0, 3, 0, 0];
        assert!(matches!(
            decode_message(&buf, true),
            Err(WireError::Incomplete(_))
        ));
    }

    #[test]
    fn valid_startup_decodes_params() {
        // Positive companion to the undersized-length guards: a well-formed startup
        // message decodes its key/value params (exercising the `pos + 1 < len` loop,
        // not just its error paths).
        let mut body = Vec::new();
        body.extend_from_slice(&196_608i32.to_be_bytes()); // protocol version 3.0
        body.extend_from_slice(b"user\0alice\0");
        body.extend_from_slice(b"database\0test\0");
        body.push(0); // terminating empty key
        let mut buf = i32::try_from(body.len() + 4)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        buf.extend_from_slice(&body);

        let (msg, consumed) = decode_message(&buf, true).unwrap();
        assert_eq!(consumed, buf.len());
        match msg {
            FrontendMessage::Startup { version, params } => {
                assert_eq!(version, 196_608);
                assert_eq!(
                    params,
                    vec![
                        ("user".to_string(), "alice".to_string()),
                        ("database".to_string(), "test".to_string()),
                    ]
                );
            }
            other => panic!("expected Startup, got {other:?}"),
        }
    }

    #[test]
    fn gss_enc_request_decodes() {
        // The 8-byte GSSENCRequest: length 8 + the GSSAPI request code. It must
        // decode to its own variant (not a Startup message), so the server can
        // decline it with `N` instead of spuriously answering AuthenticationOk.
        let mut buf = 8i32.to_be_bytes().to_vec();
        buf.extend_from_slice(&GSSAPI_ENC_REQUEST_CODE.to_be_bytes());
        let (msg, consumed) = decode_message(&buf, true).unwrap();
        assert_eq!(consumed, 8);
        assert!(matches!(msg, FrontendMessage::GssEncRequest), "{msg:?}");
    }

    #[test]
    fn cancel_request_decodes_pid_and_secret() {
        // The 16-byte CancelRequest: length 16 + the cancel code + pid + secret.
        let mut buf = 16i32.to_be_bytes().to_vec();
        buf.extend_from_slice(&CANCEL_REQUEST_CODE.to_be_bytes());
        buf.extend_from_slice(&4242i32.to_be_bytes()); // pid
        buf.extend_from_slice(&0x0102_0304i32.to_be_bytes()); // secret
        let (msg, consumed) = decode_message(&buf, true).unwrap();
        assert_eq!(consumed, 16);
        match msg {
            FrontendMessage::CancelRequest { pid, secret } => {
                assert_eq!(pid, 4242);
                assert_eq!(secret, 0x0102_0304);
            }
            other => panic!("expected CancelRequest, got {other:?}"),
        }
    }

    #[test]
    fn truncated_cancel_request_is_incomplete_not_panic() {
        // Declares length 16 but only 10 bytes present — needs more data, must not
        // read past the buffer.
        let mut buf = 16i32.to_be_bytes().to_vec();
        buf.extend_from_slice(&CANCEL_REQUEST_CODE.to_be_bytes());
        buf.extend_from_slice(&[0, 0]); // only 2 of the 8 pid/secret bytes
        assert!(matches!(
            decode_message(&buf, true),
            Err(WireError::Incomplete(_))
        ));
    }

    // --- COPY sub-protocol round-trips ----------------------------------------

    #[test]
    fn copy_in_response_encodes() {
        // overall_format 0 (text) + 2 text columns.
        let msg = BackendMessage::CopyInResponse {
            overall_format: 0,
            column_formats: vec![0, 0],
        };
        // 'G', len=11 (body 7), fmt=0, ncols=2, col0=0, col1=0.
        assert_eq!(
            encode_message(&msg).unwrap(),
            vec![b'G', 0, 0, 0, 11, 0, 0, 2, 0, 0, 0, 0]
        );
    }

    #[test]
    fn copy_out_response_encodes() {
        let msg = BackendMessage::CopyOutResponse {
            overall_format: 0,
            column_formats: vec![0],
        };
        // 'H', len=9 (body 5), fmt=0, ncols=1, col0=0.
        assert_eq!(
            encode_message(&msg).unwrap(),
            vec![b'H', 0, 0, 0, 9, 0, 0, 1, 0, 0]
        );
    }

    #[test]
    fn copy_data_and_done_encode() {
        // CopyData body is the raw payload; len = body + 4.
        assert_eq!(
            encode_message(&BackendMessage::CopyData(b"hi\n".to_vec())).unwrap(),
            vec![b'd', 0, 0, 0, 7, b'h', b'i', b'\n']
        );
        // CopyDone is an empty body.
        assert_eq!(
            encode_message(&BackendMessage::CopyDone).unwrap(),
            vec![b'c', 0, 0, 0, 4]
        );
    }

    #[test]
    fn copy_data_decodes() {
        let (msg, consumed) = decode_message(&frame(b'd', b"a\tb\n"), false).unwrap();
        assert_eq!(consumed, 4 + 5);
        match msg {
            FrontendMessage::CopyData(bytes) => assert_eq!(bytes, b"a\tb\n"),
            other => panic!("expected CopyData, got {other:?}"),
        }
        // A zero-length CopyData body is valid (empty chunk), not an error.
        let (msg, _) = decode_message(&frame(b'd', b""), false).unwrap();
        assert!(matches!(msg, FrontendMessage::CopyData(ref b) if b.is_empty()));
    }

    #[test]
    fn copy_done_decodes() {
        let (msg, _) = decode_message(&frame(b'c', b""), false).unwrap();
        assert!(matches!(msg, FrontendMessage::CopyDone));
    }

    #[test]
    fn copy_fail_decodes() {
        let (msg, _) = decode_message(&frame(b'f', b"aborted\0"), false).unwrap();
        match msg {
            FrontendMessage::CopyFail(s) => assert_eq!(s, "aborted"),
            other => panic!("expected CopyFail, got {other:?}"),
        }
    }

    #[test]
    fn copy_fail_without_terminator_errors() {
        // A CopyFail message with no NUL must be a graceful error, never a panic.
        assert!(decode_message(&frame(b'f', b"no terminator"), false).is_err());
    }

    #[test]
    fn regular_undersized_length_errors_not_panic() {
        // A complete frame whose declared length is below the 4-byte minimum:
        // `total_len < 5`, so the old `&buf[5..total_len]` body slice panicked
        // (start > end). Each must be a graceful error.
        for declared_len in [0u8, 1, 2, 3] {
            let buf = [b'Q', 0, 0, 0, declared_len, 0, 0, 0];
            assert!(
                matches!(
                    decode_message(&buf, false),
                    Err(WireError::InvalidFormat(_))
                ),
                "declared len {declared_len} must error, not panic"
            );
        }
        // len == 4 is the minimum valid (empty body) — a 'Q' with no string is a
        // graceful error from the cstring read, still no panic.
        assert!(decode_message(&[b'Q', 0, 0, 0, 4, 0], false).is_err());
    }

    #[test]
    fn startup_undersized_length_errors_not_panic() {
        // Declared length below the 8-byte minimum (header + version). len == 0
        // used to underflow `len - 1` in the params loop and panic.
        for declared_len in [0u8, 1, 7] {
            let buf = [0, 0, 0, declared_len, 0, 0, 0, 0, b'a', 0, b'b', 0];
            assert!(
                matches!(decode_message(&buf, true), Err(WireError::InvalidFormat(_))),
                "declared startup len {declared_len} must error, not panic"
            );
        }
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // Dep-free smoke fuzz: an xorshift stream of random-length, random-content
        // buffers fed to both decode paths must only ever return Ok/Err — never
        // panic (a panic here fails the test). Catches the whole class of
        // unchecked-slice / underflow bugs, not just the two known cases above.
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let n = (next() % 24) as usize;
            let buf: Vec<u8> = (0..n).map(|_| (next() & 0xff) as u8).collect();
            let _ = decode_message(&buf, false);
            let _ = decode_message(&buf, true);
        }
    }
}
