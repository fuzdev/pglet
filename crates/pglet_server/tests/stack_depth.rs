//! A statement too deep for the connection thread's stack is refused `54001` — the
//! connection, every other connection and the process all survive it — driven through
//! a real `pglet_server`.
//!
//! Without the statement-depth limit (`pglet_sql::depth`) a long flat operator chain
//! built a tree as deep as the chain was long, the first walker over it overflowed the
//! connection thread's stack, and the overflow **aborted the process**: every
//! connection dropped, and a server without `--data-dir` lost everything committed. No
//! in-process test can show that, since the abort takes the test runner with it; a
//! child process can.
//!
//! The limit is also measured here at its edge: a chain just inside
//! `SERVER_MAX_DEPTH` answers on a connection thread of `SERVER_THREAD_STACK`, in
//! whichever profile built the server — so a stack reservation too small for its limit
//! aborts this test's server rather than passing it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long a server gets to report that it is listening: a ceiling for a loaded box,
/// not an expectation.
const START_LIMIT: Duration = Duration::from_secs(30);

/// A spawned in-memory server, killed and reaped however the test leaves.
struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn spawn() -> Self {
        Self::spawn_with(&[])
    }

    /// Spawn the built server on a port the kernel picks (`--port 0`) and take the port
    /// from its own `pglet listening on …` line — the proof that this child is the one
    /// listening. A port picked in advance and probed by connecting proves neither: the
    /// port can be taken between the pick and the bind, and whatever answered on it
    /// passed for the server.
    fn spawn_with(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pglet_server"))
            .args(["--port", "0"])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn pglet_server");
        let out = child.stdout.take().expect("stdout is piped");
        let (tx, rx) = mpsc::channel();
        // Read every line, so the child never blocks on a full pipe; send the port once.
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if let Some(port) = line
                    .strip_prefix("pglet listening on 127.0.0.1:")
                    .and_then(|port| port.parse::<u16>().ok())
                {
                    let _ = tx.send(port);
                }
            }
        });
        let port = rx.recv_timeout(START_LIMIT).unwrap_or_else(|_| {
            panic!("the server did not report listening within {START_LIMIT:?}")
        });
        Self { child, port }
    }

    /// Whether the process is still running.
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A minimal simple-query protocol client: enough to send a statement and read back
/// its first value or its SQLSTATE.
struct Conn {
    stream: TcpStream,
}

/// What one simple query answered.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    /// The first column of the first row, or `None` for a statement with no rows.
    Value(Option<String>),
    /// The SQLSTATE of an `ErrorResponse`.
    Error(String),
    /// The connection closed before `ReadyForQuery`.
    Closed,
}

impl Conn {
    fn open(port: u16) -> Self {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(120)))
            .unwrap();
        let mut body = 196_608_u32.to_be_bytes().to_vec(); // protocol 3.0
        for part in ["user", "postgres", "database", "postgres", ""] {
            body.extend_from_slice(part.as_bytes());
            body.push(0);
        }
        let mut msg = u32::try_from(body.len() + 4)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        msg.extend(body);
        stream.write_all(&msg).unwrap();
        let mut conn = Self { stream };
        assert_eq!(conn.until_ready(), Reply::Value(None), "startup");
        conn
    }

    fn query(&mut self, sql: &str) -> Reply {
        let mut msg = vec![b'Q'];
        msg.extend(u32::try_from(sql.len() + 5).unwrap().to_be_bytes());
        msg.extend_from_slice(sql.as_bytes());
        msg.push(0);
        if self.stream.write_all(&msg).is_err() {
            return Reply::Closed;
        }
        self.until_ready()
    }

    /// Read messages through `ReadyForQuery`, keeping the first value or error.
    fn until_ready(&mut self) -> Reply {
        let mut reply = Reply::Value(None);
        loop {
            let mut head = [0u8; 5];
            if self.stream.read_exact(&mut head).is_err() {
                return Reply::Closed;
            }
            let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize - 4;
            let mut body = vec![0u8; len];
            if self.stream.read_exact(&mut body).is_err() {
                return Reply::Closed;
            }
            match head[0] {
                b'Z' => return reply,
                b'E' => {
                    let code = body
                        .split(|b| *b == 0)
                        .find_map(|field| field.strip_prefix(b"C"))
                        .map(|c| String::from_utf8_lossy(c).into_owned())
                        .unwrap_or_default();
                    reply = Reply::Error(code);
                }
                b'D' if reply == Reply::Value(None) => {
                    let n = i32::from_be_bytes([body[2], body[3], body[4], body[5]]);
                    let value = usize::try_from(n)
                        .ok()
                        .map(|n| String::from_utf8_lossy(&body[6..6 + n]).into_owned());
                    reply = Reply::Value(value);
                }
                _ => {}
            }
        }
    }
}

fn value(v: &str) -> Reply {
    Reply::Value(Some(v.to_owned()))
}

#[test]
fn a_statement_too_deep_is_refused_and_every_connection_survives() {
    let mut server = Server::spawn();
    let mut a = Conn::open(server.port);
    assert_eq!(a.query("CREATE TABLE t (a INT)"), Reply::Value(None));
    assert_eq!(a.query("INSERT INTO t VALUES (7)"), Reply::Value(None));

    let mut b = Conn::open(server.port);
    // Past the limit on every host: 20 000 terms is also 18.6's `54001`.
    let chain = format!("SELECT 1{}", "+1".repeat(19_999));
    assert_eq!(b.query(&chain), Reply::Error("54001".into()));

    // The connection that sent it, the one open beside it, and the process all live —
    // and so does what was committed before it, which an in-memory server lost with
    // the process.
    assert_eq!(b.query("SELECT 1"), value("1"));
    assert_eq!(a.query("SELECT a FROM t"), value("7"));
    assert!(
        server.alive(),
        "the server process must survive a too-deep statement"
    );
}

#[test]
fn the_limit_holds_at_its_edge_on_the_connection_thread() {
    let server = Server::spawn();
    let mut conn = Conn::open(server.port);
    let limit = pglet_sql::depth::SERVER_MAX_DEPTH;
    // A `+` chain is one level a term under a two-level projection, so this is just
    // inside the limit, and the one after it is past it.
    let inside = format!("SELECT (1{})::text", "+0".repeat(limit - 4));
    assert_eq!(conn.query(&inside), value("1"));
    let past = format!("SELECT (1{})::text", "+0".repeat(limit + 1));
    assert_eq!(conn.query(&past), Reply::Error("54001".into()));
    // An `AND` list is not deep at all: 18.6 answers one of 100 000 terms, and so does
    // the server.
    let list = format!(
        "SELECT count(*) FROM (VALUES (1), (2)) v(a) WHERE a = 1{}",
        " AND a = 1".repeat(99_999)
    );
    assert_eq!(conn.query(&list), value("1"));
}

/// A server reopens what its connections stored: a `CHECK` deeper than a caller's
/// thread admits (`DEFAULT_MAX_DEPTH`) but inside `SERVER_MAX_DEPTH` is decoded at
/// startup under a connection thread's bounds, not the main thread's — where it would be
/// refused, and the server would not come up.
#[test]
fn a_restarted_server_reopens_a_deep_stored_expression() {
    let dir = std::env::temp_dir().join(format!("pglet_stack_depth_srv_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data_dir = dir.to_str().unwrap().to_owned();
    let terms = pglet_sql::depth::DEFAULT_MAX_DEPTH * 2;
    {
        let server = Server::spawn_with(&["--data-dir", &data_dir]);
        let mut conn = Conn::open(server.port);
        let check = format!("CREATE TABLE c (a INT CHECK (a{} > 0))", "+1".repeat(terms));
        assert_eq!(conn.query(&check), Reply::Value(None));
        assert_eq!(conn.query("INSERT INTO c VALUES (1)"), Reply::Value(None));
    }
    let mut server = Server::spawn_with(&["--data-dir", &data_dir]);
    let mut conn = Conn::open(server.port);
    assert_eq!(conn.query("INSERT INTO c VALUES (2)"), Reply::Value(None));
    assert_eq!(conn.query("SELECT count(*) FROM c"), value("2"));
    assert!(server.alive());
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
