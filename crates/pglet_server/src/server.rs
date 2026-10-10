//! The server across a failed store: which [`SharedState`] a new connection is served
//! from, and the accept loop that asks.
//!
//! A store that fails a durable write (`pglet_storage::btree::WriteFailure`) refuses
//! every page write, save and tree-page read from then on, and every connection served
//! from it is
//! ended at its next message (`FATAL 57P02`, `handlers::handle_message`). 18.6's
//! postmaster answers a `PANIC` the same way and then recovers by itself — it ends every
//! backend, runs crash recovery, and accepts connections again (`restart_after_crash`,
//! on by default). The server does the same without a process restart: the next
//! connection it accepts is served from a state reopened from the durable slot
//! ([`Server::connection_state`]), so a supervisor never has to notice a process that is
//! alive and refusing everything.
//!
//! Reopening in the process is as sound as a restart would be. The failed store refuses
//! every page write, save and tree-page read before it touches the file
//! (`DoubleMetaStore`'s `failure.check`), so nothing of the old generation writes again,
//! and checks a tree-page read again once its bytes are read — the reopen follows the
//! failure's record, so a read that sees the reopened store's bytes sees the record, and
//! is refused rather than answered from them; a connection is
//! bound to the state it started on and never reaches the new one; and a
//! restart would read the file through the same kernel page cache this reopen reads it
//! through, since that cache outlives the process. The data-dir `flock` belongs to the
//! server, not to a state, so it is held throughout.

use std::io::{self, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, mpsc};

use parking_lot::Mutex;
use pglet::Database;
use pglet_core::file_lock::FileLock;

use crate::connection::{self, SharedState};

/// Opens the store a state is built over — the file-backed server's
/// `Database::open(data_dir)`, run on a connection thread's stack.
pub type Reopen = Box<dyn Fn() -> io::Result<Database> + Send + Sync>;

/// The server's state across a failed store: the state new connections are served
/// from, and how to reopen the store when it fails.
pub struct Server {
    /// The state a new connection is served from. Replaced — never mutated — once its
    /// store has failed, so a connection keeps the state it started on.
    current: Mutex<Arc<SharedState>>,
    /// How to open the store again from its durable slot; `None` for a server that
    /// persists nothing, whose store never fails a write.
    reopen: Option<Reopen>,
    /// Whether unsupported SQL types coerce to `TEXT`, carried to every reopened state.
    coerce_unknown_types: bool,
    /// The `lock_timeout` a connection starts with, carried to every reopened state.
    default_lock_timeout_ms: u64,
    /// Exclusive `flock` on the data dir, held for the server's lifetime — across every
    /// reopen — so a second process can't open the same `--data-dir` and corrupt its
    /// pages. Only held for its `Drop`.
    #[allow(dead_code)]
    data_dir_lock: Option<FileLock>,
}

impl Server {
    /// A server over `state`, reopened by `reopen` once its store fails.
    #[must_use]
    pub fn new(
        state: SharedState,
        reopen: Option<Reopen>,
        data_dir_lock: Option<FileLock>,
    ) -> Self {
        let coerce_unknown_types = state.coerce_unknown_types();
        let default_lock_timeout_ms = state.default_lock_timeout_ms();
        Self {
            current: Mutex::new(Arc::new(state)),
            reopen,
            coerce_unknown_types,
            default_lock_timeout_ms,
            data_dir_lock,
        }
    }

    /// A server over the single-file store under `data_dir`, which survives restarts.
    ///
    /// Takes the data-dir lock before touching any file, so a second process can't open
    /// the same directory, then opens the store — `Database::open`, on a connection
    /// thread's stack, since opening decodes every stored expression a connection's
    /// bounds admitted (`pglet_sql::depth`).
    ///
    /// # Errors
    ///
    /// The lock held elsewhere, or the store's open error.
    pub fn with_file_btree(
        data_dir: &Path,
        coerce_unknown_types: bool,
        default_lock_timeout_ms: u64,
    ) -> io::Result<Self> {
        let data_dir_lock = FileLock::acquire(&data_dir.join("pglet.lock"))
            .map_err(|e| io::Error::other(format!("data dir locked: {e}")))?;
        let dir: PathBuf = data_dir.to_path_buf();
        let reopen: Reopen = Box::new(move || {
            let dir = dir.clone();
            on_connection_stack(move || Database::open(&dir))?
        });
        let state = SharedState::from_database(reopen()?)
            .with_coerce_unknown_types(coerce_unknown_types)
            .with_default_lock_timeout(default_lock_timeout_ms);
        Ok(Self::new(state, Some(reopen), Some(data_dir_lock)))
    }

    /// The state a new connection is served from.
    ///
    /// The current one, until its store has failed a durable write; then a state over
    /// the store reopened from its durable slot, which replaces it — every connection
    /// still on the failed one is ended at its next message, and every one waiting for
    /// the failed state's writer is woken and answered `57P02`
    /// (`crate::writer::WriterLock::fail_waiters`). A reopen that fails leaves the failed
    /// state current, which refuses the connection's startup (`FATAL 57P03`), and the
    /// next connection tries again.
    pub fn connection_state(&self) -> Arc<SharedState> {
        let mut current = self.current.lock();
        if let (Some(reason), Some(reopen)) = (current.write_failure_reason(), &self.reopen) {
            match reopen() {
                Ok(db) => {
                    eprintln!(
                        "pglet: a durable write failed ({reason}); reopened the store from its durable state"
                    );
                    current.writer.fail_waiters(reason);
                    *current = Arc::new(
                        SharedState::from_database(db)
                            .with_coerce_unknown_types(self.coerce_unknown_types)
                            .with_default_lock_timeout(self.default_lock_timeout_ms),
                    );
                }
                Err(e) => eprintln!(
                    "pglet: a durable write failed ({reason}); reopening the store failed: {e}"
                ),
            }
        }
        Arc::clone(&current)
    }
}

/// Process ids handed to connections, one each.
static NEXT_PID: AtomicI32 = AtomicI32::new(1);

/// Accept connections on `listener` for as long as it yields them, each on its own
/// thread and served from [`Server::connection_state`].
pub fn serve(listener: &TcpListener, server: &Server) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                // Disable Nagle, as Postgres does for every TCP backend socket
                // (`pqcomm.c`'s `pq_init`, which treats the failure as FATAL).
                // Not a tuning knob — a response larger than the connection loop's
                // `BufWriter` still reaches the socket as several writes with no read
                // between them, the last a small one behind unacknowledged data, which
                // is the exact shape Nagle holds for the peer's ACK (reasoned from the
                // loop, not re-measured on it); the peer has nothing to send, so its delayed-ACK timer (~40 ms) fires
                // instead. Measured when the loop wrote each protocol message
                // separately, before the buffer: on three fuz_app `db` files, 68 tests
                // took **19.66 s** with Nagle on and **0.265 s** with it off,
                // red-on-revert both ways. A silently
                // Nagle'd connection is the silent-wrong class this engine refuses
                // elsewhere, so a failure here loses the connection rather than
                // serving it 70× slow.
                if let Err(e) = stream.set_nodelay(true) {
                    eprintln!("connection error: cannot set TCP_NODELAY: {e}");
                    continue;
                }
                let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
                let state = server.connection_state();
                // The connection thread's stack and the statement-tree depth it admits
                // are one budget (`pglet_sql::depth`): the thread is sized for
                // `SERVER_MAX_DEPTH`, and a deeper statement is `54001` rather than an
                // overflow that aborts every connection.
                //
                // The stream is handed over once the thread exists, so a failed spawn
                // leaves it here to be refused — that connection, not the server.
                let (hand_off, receive) = mpsc::channel::<TcpStream>();
                let spawned = std::thread::Builder::new()
                    .stack_size(pglet_sql::depth::SERVER_THREAD_STACK)
                    .spawn(move || {
                        let Ok(stream) = receive.recv() else {
                            return;
                        };
                        pglet_sql::depth::set_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
                        pglet_sql::depth::set_thread_stack(pglet_sql::depth::SERVER_THREAD_STACK);
                        if let Err(e) = connection::handle_connection(stream, pid, state) {
                            eprintln!("connection error: {e}");
                        }
                    });
                match spawned {
                    // The thread blocks in `recv` until this send, so it cannot fail.
                    Ok(_) => drop(hand_off.send(stream)),
                    Err(e) => refuse_connection(stream, &e),
                }
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

/// Run `f` on a thread with a connection thread's stack and depth bounds
/// (`pglet_sql::depth`): what a connection stored, only a thread as deep can read back.
///
/// # Errors
///
/// The thread could not be spawned, or it panicked.
pub fn on_connection_stack<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> io::Result<T> {
    std::thread::Builder::new()
        .stack_size(pglet_sql::depth::SERVER_THREAD_STACK)
        .spawn(move || {
            pglet_sql::depth::set_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
            pglet_sql::depth::set_thread_stack(pglet_sql::depth::SERVER_THREAD_STACK);
            f()
        })?
        .join()
        .map_err(|_| io::Error::other("the opening thread panicked"))
}

/// Refuse a connection no thread could be spawned for, and keep serving the others.
///
/// 18.6's answer when it cannot fork a backend (`report_fork_failure_to_client`): one
/// protocol-2 error packet — `E`, the sentence, a newline and a NUL, no SQLSTATE —
/// sent once on a non-blocking socket before the startup packet is read, then the
/// socket closes. libpq reports the sentence as the connection failure.
fn refuse_connection(mut stream: TcpStream, error: &io::Error) {
    eprintln!("connection refused: cannot spawn a connection thread: {error}");
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    // `strerror`'s text: `io::Error` appends ` (os error N)` to it.
    let text = error.to_string();
    let reason = text
        .rsplit_once(" (os error ")
        .map_or(text.as_str(), |(r, _)| r);
    let packet = format!("Ecould not fork new process for connection: {reason}\n\0");
    let _ = stream.write(packet.as_bytes());
}

#[cfg(test)]
mod tests {
    //! A server whose store fails a durable write returns to service by itself, over a
    //! real loopback socket through [`serve`]: every connection on the failed store is
    //! ended, and a reconnect is served from the durable state — with the data-dir lock
    //! held throughout.

    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use pglet_core::file_lock::FileLock;
    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::{ScratchDir, open_database};

    use super::{Reopen, Server, serve};
    use crate::connection::SharedState;

    /// A minimal protocol-3 client: a startup, then simple queries.
    struct Client {
        stream: TcpStream,
    }

    /// A query's answer: the rows as text, or the error's SQLSTATE and whether it was
    /// `FATAL`.
    type Answer = Result<Vec<Vec<Option<String>>>, (String, bool)>;

    impl Client {
        /// Connect and send a startup; `Err` carries the startup's error.
        fn connect(addr: SocketAddr) -> Result<Self, (String, bool)> {
            let stream = TcpStream::connect(addr).expect("connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .expect("read timeout");
            let mut client = Self { stream };
            let params: &[u8] = b"user\0postgres\0\0";
            let mut packet = i32::try_from(8 + params.len())
                .unwrap()
                .to_be_bytes()
                .to_vec();
            packet.extend_from_slice(&196_608i32.to_be_bytes());
            packet.extend_from_slice(params);
            client.stream.write_all(&packet).expect("startup");
            client.answer().map(|_| client)
        }

        /// Send `sql` as a simple query and read its answer.
        fn query(&mut self, sql: &str) -> Answer {
            let mut packet = vec![b'Q'];
            packet.extend_from_slice(&i32::try_from(4 + sql.len() + 1).unwrap().to_be_bytes());
            packet.extend_from_slice(sql.as_bytes());
            packet.push(0);
            if self.stream.write_all(&packet).is_err() {
                return Err(("closed".into(), true));
            }
            self.answer()
        }

        /// One backend frame, or `None` at the end of the stream.
        fn frame(&mut self) -> Option<(u8, Vec<u8>)> {
            let mut head = [0u8; 5];
            self.stream.read_exact(&mut head).ok()?;
            let len =
                usize::try_from(i32::from_be_bytes([head[1], head[2], head[3], head[4]])).unwrap();
            let mut body = vec![0u8; len - 4];
            self.stream.read_exact(&mut body).ok()?;
            Some((head[0], body))
        }

        /// Read frames to `ReadyForQuery`, or to the end of the stream after a `FATAL`.
        fn answer(&mut self) -> Answer {
            let mut rows = Vec::new();
            let mut error: Option<(String, bool)> = None;
            while let Some((tag, body)) = self.frame() {
                match tag {
                    b'D' => {
                        let mut at = 2;
                        let mut row = Vec::new();
                        for _ in 0..i16::from_be_bytes([body[0], body[1]]) {
                            let len = i32::from_be_bytes(body[at..at + 4].try_into().unwrap());
                            at += 4;
                            if let Ok(len) = usize::try_from(len) {
                                row.push(Some(String::from_utf8_lossy(&body[at..at + len]).into()));
                                at += len;
                            } else {
                                row.push(None);
                            }
                        }
                        rows.push(row);
                    }
                    b'E' => {
                        let fields: Vec<(u8, String)> = body
                            .split(|&b| b == 0)
                            .filter(|f| !f.is_empty())
                            .map(|f| (f[0], String::from_utf8_lossy(&f[1..]).into()))
                            .collect();
                        let field = |k: u8| {
                            fields
                                .iter()
                                .find(|(c, _)| *c == k)
                                .map(|(_, v)| v.clone())
                                .unwrap_or_default()
                        };
                        error = Some((field(b'C'), field(b'V') == "FATAL"));
                    }
                    b'Z' => break,
                    _ => {}
                }
            }
            error.map_or(Ok(rows), Err)
        }

        /// Whether the server has closed the connection.
        fn is_closed(&mut self) -> bool {
            self.frame().is_none()
        }
    }

    /// The ids `SELECT id FROM t ORDER BY id` answers.
    fn ids(client: &mut Client) -> Result<Vec<String>, (String, bool)> {
        client.query("SELECT id FROM t ORDER BY id").map(|rows| {
            rows.into_iter()
                .map(|r| r[0].clone().unwrap_or_default())
                .collect()
        })
    }

    /// A server over the store on `io`, reopened by `reopen`, holding the lock on
    /// `lock_path`, serving on a loopback port from a thread of its own.
    fn spawn(io: &MemBlockIo, reopen: Reopen, lock_path: &std::path::Path) -> SocketAddr {
        let state = SharedState::from_database(open_database(Box::new(io.clone())));
        let lock = FileLock::acquire(lock_path).expect("the data-dir lock");
        let server = Server::new(state, Some(reopen), Some(lock));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || serve(&listener, &server));
        addr
    }

    /// After a durable write fails, the server returns to service with no operator, as
    /// 18.6's postmaster does after a `PANIC`: the connection that met the failure and
    /// an idle one in a block are each ended (`FATAL 57P02`) — the idle one after the
    /// reopen, so a connection is not served from a state it did not start on — and a
    /// reconnect reads exactly the durable rows and writes durably. The data-dir lock is
    /// held throughout.
    #[test]
    fn a_failed_store_is_reopened_for_the_next_connection() {
        let scratch = ScratchDir::new("server_reopen");
        let lock_path = scratch.path.join("pglet.lock");
        let io = MemBlockIo::new();
        let reopened = io.clone();
        let reopen: Reopen = Box::new(move || Ok(open_database(Box::new(reopened.reopen()))));
        let addr = spawn(&io, reopen, &lock_path);

        let mut a = Client::connect(addr).expect("startup");
        a.query("CREATE TABLE t (id INT)").unwrap();
        a.query("INSERT INTO t VALUES (1)").unwrap();
        let mut idle = Client::connect(addr).expect("startup");
        idle.query("BEGIN").unwrap();
        assert_eq!(ids(&mut idle), Ok(vec!["1".to_string()]));

        let policy = io.policy();
        policy.arm_fail_slot_writes();
        let failed = a.query("INSERT INTO t VALUES (2)");
        policy.disarm();
        assert_eq!(failed, Err(("58030".into(), false)), "the failed write");
        assert!(
            FileLock::acquire(&lock_path).is_err(),
            "the data-dir lock is held after the failure"
        );

        // The reconnect reopens the store from its durable slot.
        let mut c = Client::connect(addr).expect("a reconnect is served");
        assert_eq!(ids(&mut c), Ok(vec!["1".to_string()]), "the durable rows");
        assert!(
            FileLock::acquire(&lock_path).is_err(),
            "the data-dir lock is held after the reopen"
        );

        // Each connection on the failed store is ended, not served from the new one.
        assert_eq!(
            ids(&mut idle),
            Err(("57P02".into(), true)),
            "the idle block"
        );
        assert!(idle.is_closed(), "the idle connection ends");
        assert_eq!(
            ids(&mut a),
            Err(("57P02".into(), true)),
            "the failed connection"
        );
        assert!(a.is_closed(), "the failed connection ends");

        c.query("INSERT INTO t VALUES (3)").unwrap();
        assert_eq!(ids(&mut c), Ok(vec!["1".to_string(), "3".to_string()]));
        let mut durable = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
        let rows = durable.execute("SELECT id FROM t ORDER BY id").unwrap();
        assert_eq!(
            rows[0].rows,
            vec![
                vec![pglet_sql::types::Value::Int4(1)],
                vec![pglet_sql::types::Value::Int4(3)]
            ],
            "the reopened store's write is durable"
        );
    }

    /// A reopen that fails leaves the failed store current: that connection's startup
    /// is refused `FATAL 57P03`, and the next connection tries the reopen again.
    #[test]
    fn a_failed_reopen_refuses_the_startup_and_is_tried_again() {
        let scratch = ScratchDir::new("server_reopen_retry");
        let io = MemBlockIo::new();
        let reopened = io.clone();
        let tries = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&tries);
        let reopen: Reopen = Box::new(move || {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(std::io::Error::other("the device is still failing"))
            } else {
                Ok(open_database(Box::new(reopened.reopen())))
            }
        });
        let addr = spawn(&io, reopen, &scratch.path.join("pglet.lock"));

        let mut a = Client::connect(addr).expect("startup");
        a.query("CREATE TABLE t (id INT)").unwrap();
        a.query("INSERT INTO t VALUES (1)").unwrap();
        let policy = io.policy();
        policy.arm_fail_slot_writes();
        assert!(a.query("INSERT INTO t VALUES (2)").is_err());
        policy.disarm();

        assert_eq!(
            Client::connect(addr).err(),
            Some(("57P03".into(), true)),
            "a startup while the reopen fails"
        );
        let mut c = Client::connect(addr).expect("the next reconnect is served");
        assert_eq!(ids(&mut c), Ok(vec!["1".to_string()]));
        assert_eq!(
            tries.load(Ordering::SeqCst),
            2,
            "one reopen per connection until one opens"
        );
    }
}
