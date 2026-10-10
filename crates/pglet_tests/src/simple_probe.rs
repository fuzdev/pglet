//! A minimal Postgres frontend for the **simple query protocol**, used by the
//! differential harness's text leg.
//!
//! It exists for one thing tokio-postgres cannot do: read the *declared column
//! types* of a simple query. `tokio_postgres::SimpleColumn` carries only a name, so
//! the `RowDescription` a `Query` message replies with — the server's declared type
//! for every column — is discarded before the client hands the rows over. That made
//! the whole simple-query declared-type surface untestable, and pglet computes it
//! through **different code** than the extended protocol does (the executor's
//! `QueryResult::shape` vs the planner's `Plan::result_shape`), so a divergence
//! there is invisible to the extended-protocol leg. Speaking the protocol directly
//! is the only way to see it.
//!
//! The **extended**-protocol half of that comparison needs no probe: a prepared
//! statement's `Describe` reply is exactly what `tokio_postgres::Statement::columns`
//! carries, so the harness reads the planner walker's declaration straight off it
//! (see `execute_test`). Only the simple query throws its `RowDescription` away.
//!
//! Deliberately small: startup with trust auth (the harness's Postgres and every
//! spawned pglet server take it), one statement per `Query`, and just enough of the
//! backend message set to reach `ReadyForQuery`. It is a test instrument, not a
//! client — no TLS, no auth methods, no pipelining, no `COPY`.

use crate::{DbConfig, DeclaredColumn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Postgres protocol version 3.0, as the startup message encodes it.
const PROTOCOL_VERSION_3: i32 = 196_608;

/// The `AuthenticationRequest` sub-code's method name, for the refusal message.
///
/// The codes are `protocol.h`'s `AUTH_REQ_*`. An unrecognized one is reported as
/// its number rather than guessed at.
fn auth_method_name(code: i32) -> String {
    match code {
        2 => "Kerberos V5".to_owned(),
        3 => "cleartext password".to_owned(),
        5 => "MD5 password".to_owned(),
        7 => "GSSAPI".to_owned(),
        9 => "SSPI".to_owned(),
        10 => "SASL (SCRAM)".to_owned(),
        other => format!("an unrecognized (code {other})"),
    }
}

/// One statement's simple-query reply: the declared columns and the raw text of
/// every value the server sent (`None` for SQL NULL).
#[derive(Debug, Clone, Default)]
pub struct ProbeOutput {
    pub columns: Vec<DeclaredColumn>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// A probe failure, split by **whose** fault it is.
///
/// The split is load-bearing for `@expect-error` fixtures: a `Db` error is the
/// engine answering, which such a fixture may legitimately assert, while a
/// `Client` error means the harness itself could not read the reply — a broken
/// instrument, never evidence about the engine. Scoring the two alike is how a
/// harness bug passes for a passing test.
#[derive(Debug, Clone)]
pub enum ProbeError {
    /// The server raised an error: its SQLSTATE and message.
    Db { code: String, message: String },
    /// The connection or the reply could not be read/parsed.
    Client(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db { code, message } => write!(f, "{code}: {message}"),
            Self::Client(m) => write!(f, "client: {m}"),
        }
    }
}

/// A connection speaking the simple query protocol by hand. See the module docs.
#[derive(Debug)]
pub struct SimpleProbe {
    stream: TcpStream,
    /// Unconsumed bytes from the last read — a backend message can straddle reads.
    buf: Vec<u8>,
}

impl SimpleProbe {
    /// Open a connection and complete startup (trust auth only).
    ///
    /// # Errors
    ///
    /// [`ProbeError::Client`] when the socket can't be opened, the server demands an
    /// authentication method beyond trust, or startup doesn't reach `ReadyForQuery`;
    /// [`ProbeError::Db`] when the server refuses the startup outright.
    pub async fn connect(config: &DbConfig) -> Result<Self, ProbeError> {
        let stream = TcpStream::connect((config.host, config.port))
            .await
            .map_err(|e| ProbeError::Client(format!("connect: {e}")))?;
        let mut probe = Self {
            stream,
            buf: Vec::new(),
        };
        probe.send_startup(config.user, config.database).await?;
        probe.finish_startup().await?;
        // Pin the session to UTC, exactly as `connect` does for the binary leg. pglet
        // has no session zone (it always renders UTC), so this only moves Postgres —
        // and without it a `timestamptz` renders in the server's local zone and the
        // text comparison is against a different instant's spelling. The contract the
        // harness asserts is pglet ≡ Postgres at `TimeZone=UTC`, on **both** legs.
        probe.query("SET TIME ZONE 'UTC'").await?;
        Ok(probe)
    }

    /// Run one statement and return its declared columns + text values.
    ///
    /// Mirrors the harness's one-statement-per-message discipline. A statement that
    /// returns no rows yields an empty [`ProbeOutput`].
    ///
    /// # Errors
    ///
    /// [`ProbeError::Db`] for a server error (SQLSTATE + message), [`ProbeError::Client`]
    /// for an I/O or parse failure.
    pub async fn query(&mut self, sql: &str) -> Result<ProbeOutput, ProbeError> {
        let mut body = Vec::with_capacity(sql.len() + 1);
        body.extend_from_slice(sql.as_bytes());
        body.push(0);
        self.send_message(b'Q', &body).await?;
        self.read_until_ready().await
    }

    // ---- startup -----------------------------------------------------------

    async fn send_startup(&mut self, user: &str, database: &str) -> Result<(), ProbeError> {
        let mut body = Vec::new();
        body.extend_from_slice(&PROTOCOL_VERSION_3.to_be_bytes());
        for (key, value) in [("user", user), ("database", database)] {
            body.extend_from_slice(key.as_bytes());
            body.push(0);
            body.extend_from_slice(value.as_bytes());
            body.push(0);
        }
        body.push(0);
        // The startup message has no type byte; its length covers itself.
        let len = i32::try_from(body.len() + 4)
            .map_err(|_| ProbeError::Client("startup message too large".into()))?;
        let mut frame = Vec::with_capacity(body.len() + 4);
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&body);
        self.stream
            .write_all(&frame)
            .await
            .map_err(|e| ProbeError::Client(format!("write startup: {e}")))
    }

    /// Consume the startup reply through `ReadyForQuery`, rejecting any auth
    /// method beyond `AuthenticationOk`.
    async fn finish_startup(&mut self) -> Result<(), ProbeError> {
        loop {
            let (tag, body) = self.read_message().await?;
            match tag {
                // AuthenticationRequest: only code 0 (Ok) is supported.
                b'R' => {
                    let code = read_i32(&body, 0)?;
                    if code != 0 {
                        // Named specifically, and naming the oracle knob, because
                        // this is where a password in `PGLET_PG_URL` stops being
                        // honoured: the tokio-postgres leg and the `psql` /
                        // `pg_dump` shell-outs authenticate with it and this probe
                        // cannot, so a password-demanding oracle fails *only* the
                        // text-format leg — every fixture's, counted as a failure.
                        // A generic "unsupported authentication request 10" left
                        // the reader to discover that.
                        return Err(ProbeError::Client(format!(
                            "the server asked for {} authentication; the probe speaks trust auth \
                             only, so the differential harness's text-format leg needs a server \
                             that lets this user in without a password (a password in {} reaches \
                             the tokio-postgres leg and the psql/pg_dump shell-outs, not this one)",
                            auth_method_name(code),
                            crate::ORACLE_URL_VAR,
                        )));
                    }
                }
                b'E' => return Err(parse_error_response(&body)),
                b'Z' => return Ok(()),
                // ParameterStatus / BackendKeyData / NoticeResponse — nothing to do.
                _ => {}
            }
        }
    }

    // ---- reply parsing -----------------------------------------------------

    /// Drain the reply to one `Query`, keeping the **last** `RowDescription` and its
    /// rows (the harness's "last statement's rows are the result" rule).
    ///
    /// A server error is returned as soon as it is seen, but the stream is still
    /// drained to `ReadyForQuery` first so the connection stays usable.
    async fn read_until_ready(&mut self) -> Result<ProbeOutput, ProbeError> {
        let mut out = ProbeOutput::default();
        let mut error: Option<ProbeError> = None;
        loop {
            let (tag, body) = self.read_message().await?;
            match tag {
                b'T' => {
                    out.columns = parse_row_description(&body)?;
                    out.rows.clear();
                }
                b'D' => out.rows.push(parse_data_row(&body)?),
                b'E' => error = Some(parse_error_response(&body)),
                b'Z' => break,
                // CommandComplete / EmptyQueryResponse / ParameterStatus / Notice —
                // nothing to collect; a later statement's `T` supersedes the last.
                _ => {}
            }
        }
        error.map_or(Ok(out), Err)
    }

    async fn send_message(&mut self, tag: u8, body: &[u8]) -> Result<(), ProbeError> {
        let len = i32::try_from(body.len() + 4)
            .map_err(|_| ProbeError::Client("message too large".into()))?;
        let mut frame = Vec::with_capacity(body.len() + 5);
        frame.push(tag);
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(body);
        self.stream
            .write_all(&frame)
            .await
            .map_err(|e| ProbeError::Client(format!("write: {e}")))
    }

    /// Read one backend message: its type byte and its body (length prefix stripped).
    async fn read_message(&mut self) -> Result<(u8, Vec<u8>), ProbeError> {
        self.fill_to(5).await?;
        let tag = self.buf[0];
        let len = read_i32(&self.buf, 1)?;
        let total = usize::try_from(len)
            .map_err(|_| ProbeError::Client(format!("negative message length {len}")))?
            + 1;
        if total < 5 {
            return Err(ProbeError::Client(format!(
                "message length {len} too short"
            )));
        }
        self.fill_to(total).await?;
        let body = self.buf[5..total].to_vec();
        self.buf.drain(..total);
        Ok((tag, body))
    }

    /// Read until the buffer holds at least `n` bytes.
    async fn fill_to(&mut self, n: usize) -> Result<(), ProbeError> {
        while self.buf.len() < n {
            let mut chunk = [0u8; 8192];
            let read = self
                .stream
                .read(&mut chunk)
                .await
                .map_err(|e| ProbeError::Client(format!("read: {e}")))?;
            if read == 0 {
                return Err(ProbeError::Client("connection closed mid-message".into()));
            }
            self.buf.extend_from_slice(&chunk[..read]);
        }
        Ok(())
    }
}

/// Parse a `RowDescription` body: `Int16` field count, then per field a name,
/// table OID, attnum, **type OID**, type length, typmod, and format code.
fn parse_row_description(body: &[u8]) -> Result<Vec<DeclaredColumn>, ProbeError> {
    let count = read_i16(body, 0)?;
    let mut pos = 2;
    let mut columns = Vec::new();
    for _ in 0..count.max(0) {
        let (name, next) = read_cstring(body, pos)?;
        // name, then: table OID (4) + attnum (2), type OID (4), typlen (2),
        // typmod (4), format (2).
        let type_oid = read_i32(body, next + 6)?;
        pos = next + 18;
        columns.push(DeclaredColumn {
            name,
            #[allow(clippy::cast_sign_loss)] // OIDs are unsigned on the wire
            type_oid: type_oid as u32,
        });
    }
    Ok(columns)
}

/// Parse a `DataRow` body: `Int16` column count, then per column a length
/// (`-1` = SQL NULL) and that many bytes of **text**.
fn parse_data_row(body: &[u8]) -> Result<Vec<Option<String>>, ProbeError> {
    let count = read_i16(body, 0)?;
    let mut pos = 2;
    let mut values = Vec::new();
    for _ in 0..count.max(0) {
        let len = read_i32(body, pos)?;
        pos += 4;
        if len < 0 {
            values.push(None);
            continue;
        }
        let len = usize::try_from(len)
            .map_err(|_| ProbeError::Client("bad data-row column length".into()))?;
        let bytes = body
            .get(pos..pos + len)
            .ok_or_else(|| ProbeError::Client("data row truncated".into()))?;
        values.push(Some(String::from_utf8_lossy(bytes).into_owned()));
        pos += len;
    }
    Ok(values)
}

/// Parse an `ErrorResponse` body into its SQLSTATE (`C`) and message (`M`).
fn parse_error_response(body: &[u8]) -> ProbeError {
    let mut code = String::new();
    let mut message = String::new();
    let mut pos = 0;
    while let Some(&field) = body.get(pos) {
        if field == 0 {
            break;
        }
        let Ok((value, next)) = read_cstring(body, pos + 1) else {
            break;
        };
        match field {
            b'C' => code = value,
            b'M' => message = value,
            _ => {}
        }
        pos = next;
    }
    ProbeError::Db { code, message }
}

fn read_i16(body: &[u8], pos: usize) -> Result<i16, ProbeError> {
    body.get(pos..pos + 2)
        .and_then(|s| s.try_into().ok())
        .map(i16::from_be_bytes)
        .ok_or_else(|| ProbeError::Client("truncated i16".into()))
}

fn read_i32(body: &[u8], pos: usize) -> Result<i32, ProbeError> {
    body.get(pos..pos + 4)
        .and_then(|s| s.try_into().ok())
        .map(i32::from_be_bytes)
        .ok_or_else(|| ProbeError::Client("truncated i32".into()))
}

/// Read a null-terminated string starting at `pos`, returning it and the index
/// just past its terminator.
fn read_cstring(body: &[u8], pos: usize) -> Result<(String, usize), ProbeError> {
    let end = body
        .get(pos..)
        .and_then(|rest| rest.iter().position(|b| *b == 0))
        .ok_or_else(|| ProbeError::Client("unterminated string".into()))?
        + pos;
    Ok((
        String::from_utf8_lossy(&body[pos..end]).into_owned(),
        end + 1,
    ))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "a failed setup in a test is a test failure, and the message is the diagnosis"
)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Stand up a socket that answers the startup message with one
    /// `AuthenticationRequest` carrying `code`, and return the probe's refusal.
    ///
    /// A real server demanding a password needs a `pg_hba.conf` the harness has no
    /// business editing; the refusal is decided by that one message, so the message
    /// is what the test sends.
    async fn refusal_for_auth_request(code: i32) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("a loopback port is available");
        let port = listener
            .local_addr()
            .expect("a bound listener has an address")
            .port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("the probe connects");
            let mut startup = [0_u8; 512];
            let _ = stream.read(&mut startup).await;
            let mut frame = vec![b'R'];
            frame.extend_from_slice(&8_i32.to_be_bytes());
            frame.extend_from_slice(&code.to_be_bytes());
            let _ = stream.write_all(&frame).await;
        });
        let config = DbConfig {
            host: "127.0.0.1",
            port,
            user: "alice",
            database: "scratch",
            password: Some("s3cret"),
        };
        let error = SimpleProbe::connect(&config)
            .await
            .expect_err("an authentication request is refused");
        server.await.expect("the fake server finishes");
        error.to_string()
    }

    /// The refusal is the probe's whole answer to a password-demanding oracle, and
    /// what it has to say is *which* leg died and *which* knob is involved — a bare
    /// "unsupported authentication request 10" left the reader to find that out.
    /// Nothing else measures this path: the harness's own oracle takes trust auth.
    #[tokio::test]
    async fn a_password_demanding_server_is_refused_by_method_and_by_knob() {
        for (code, method) in [(10, "SASL"), (5, "MD5"), (3, "cleartext password")] {
            let message = refusal_for_auth_request(code).await;
            assert!(
                message.contains(method),
                "code {code} should name {method}: {message}"
            );
            assert!(
                message.contains(crate::ORACLE_URL_VAR),
                "code {code} should name the oracle knob: {message}"
            );
            assert!(
                message.contains("text-format"),
                "code {code} should say which leg it kills: {message}"
            );
        }
    }

    /// An unrecognized sub-code is reported as its number rather than guessed at:
    /// naming it after a neighbouring method would be a confident wrong answer.
    #[test]
    fn an_unrecognized_auth_method_is_reported_as_its_code() {
        assert_eq!(auth_method_name(10), "SASL (SCRAM)");
        assert_eq!(auth_method_name(12), "an unrecognized (code 12)");
    }
}
