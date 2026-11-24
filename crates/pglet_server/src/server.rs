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
//! server and to every connection it serves, not to a state: each connection thread holds
//! a share of it for as long as it runs, so the lock is held across every reopen and is
//! released only once the server and the last connection that can still write the store
//! are gone — a statement still running after a spawned server's handle drops keeps
//! another opener out until it ends.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write as _};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;

use parking_lot::Mutex;
use pglet::Database;
use pglet_core::file_lock::FileLock;

use crate::connection::{self, SharedState};

/// Opens the store a state is built over — the file-backed server's
/// `Database::open(data_dir)`, run on a connection thread's stack.
pub type Reopen = Box<dyn Fn() -> io::Result<Database> + Send + Sync>;

/// The storage backend a server runs on — `--storage`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StorageKind {
    /// The CoW B+tree with MVCC, in memory unless [`ServerOptions::data_dir`] makes it
    /// persistent. The default, and what a test suite wants: `Memory` has no MVCC, and a
    /// failed autocommit statement's partial effects stand there (`EDGES.md` row 40).
    #[default]
    Btree,
    /// `MemoryStorage`: no MVCC and no persistence.
    Memory,
}

/// What a server is opened with — the binary's flags, less the port and the `dump`
/// subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOptions {
    /// The storage backend (`--storage`); the B+tree by default.
    pub storage: StorageKind,
    /// Persist the B+tree to this directory (`--data-dir`); `None` keeps it in memory.
    /// [`StorageKind::Memory`] refuses one.
    pub data_dir: Option<PathBuf>,
    /// Coerce a type name pglet does not model to `TEXT` instead of refusing it
    /// (`--coerce-unknown-types`).
    pub coerce_unknown_types: bool,
    /// How long a write waits for the writer before `55P03`, in milliseconds; `0` waits
    /// without a bound (`--lock-timeout`).
    pub lock_timeout_ms: u64,
}

impl Default for ServerOptions {
    /// An in-memory B+tree, unknown types refused, the default `lock_timeout`.
    fn default() -> Self {
        Self {
            storage: StorageKind::Btree,
            data_dir: None,
            coerce_unknown_types: false,
            lock_timeout_ms: pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS,
        }
    }
}

/// The portable SQL dump of the database persisted under `data_dir` — the binary's `dump`
/// subcommand (`pglet::Database::dump_sql`).
///
/// The store is opened with `open_existing`, so a mistyped directory is an error rather
/// than an empty store's near-empty dump, and read on a connection thread's stack, so it
/// exports a store an embedded opener refuses (`54001`).
///
/// # Errors
///
/// The store does not open, or the dump fails.
pub fn dump_sql(data_dir: &Path) -> io::Result<String> {
    let dir = data_dir.to_path_buf();
    on_connection_stack(move || {
        let db = Database::open_existing(&dir).map_err(|e| io::Error::other(e.to_string()))?;
        db.dump_sql().map_err(|e| io::Error::other(e.to_string()))
    })?
}

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
    /// Exclusive `flock` on the data dir, so a second opener — another process, or
    /// another [`Server::open`] in this one — can't open the same directory and corrupt
    /// its pages. Shared, not owned: [`serve`] moves a clone into every connection thread,
    /// so the lock outlives the server for as long as a connection can still write the
    /// store, and it is released when the last share drops.
    data_dir_lock: Option<Arc<FileLock>>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server")
            .field("persistent", &self.reopen.is_some())
            .field("coerce_unknown_types", &self.coerce_unknown_types)
            .field("default_lock_timeout_ms", &self.default_lock_timeout_ms)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Open a server's state as `options` asks: an in-memory B+tree, `MemoryStorage`, or
    /// the single-file B+tree under a data dir, whose lock it and every connection it serves then hold, until the last of them ends.
    ///
    /// # Errors
    ///
    /// [`StorageKind::Memory`] with a data dir (`InvalidInput`), the data-dir lock held
    /// elsewhere, or the store's open error.
    pub fn open(options: &ServerOptions) -> io::Result<Self> {
        let ServerOptions {
            storage,
            data_dir,
            coerce_unknown_types,
            lock_timeout_ms,
        } = options;
        let in_memory = |state: SharedState| {
            Self::new(
                state
                    .with_coerce_unknown_types(*coerce_unknown_types)
                    .with_default_lock_timeout(*lock_timeout_ms),
                None,
                None,
            )
        };
        match (storage, data_dir) {
            (StorageKind::Memory, Some(_)) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "memory storage keeps nothing; it cannot take a data dir",
            )),
            (StorageKind::Memory, None) => Ok(in_memory(SharedState::in_memory())),
            (StorageKind::Btree, None) => Ok(in_memory(SharedState::with_btree())),
            (StorageKind::Btree, Some(dir)) => {
                Self::with_file_btree(dir, *coerce_unknown_types, *lock_timeout_ms)
            }
        }
    }

    /// Accept connections on `listener` for as long as it yields them, on this thread —
    /// the binary's foreground loop. Each connection is served on a thread of its own.
    pub fn serve(&self, listener: &TcpListener) {
        serve(listener, self, None);
    }

    /// Serve on `listener` from a thread of its own until the returned handle is dropped
    /// or [`ServerHandle::shutdown`] — how a test suite runs the server in its process.
    ///
    /// # Errors
    ///
    /// The listener's address cannot be read, or the accept thread cannot be spawned.
    pub fn spawn(self, listener: TcpListener) -> io::Result<ServerHandle> {
        let local_addr = listener.local_addr()?;
        let stop = Arc::new(Stop::default());
        let accept_stop = Arc::clone(&stop);
        let accept = std::thread::Builder::new()
            .name("pglet_server accept".into())
            .spawn(move || serve(&listener, &self, Some(&accept_stop)))?;
        Ok(ServerHandle {
            local_addr,
            stop,
            accept: Some(accept),
        })
    }

    /// [`Server::spawn`] on a listener bound to `127.0.0.1` at a port the kernel picks,
    /// which [`ServerHandle::local_addr`] reports.
    ///
    /// # Errors
    ///
    /// The bind fails, or [`Server::spawn`]'s errors.
    pub fn spawn_loopback(self) -> io::Result<ServerHandle> {
        self.spawn(TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?)
    }

    /// A server over `state`, reopened by `reopen` once its store fails.
    #[must_use]
    pub(crate) fn new(
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
            data_dir_lock: data_dir_lock.map(Arc::new),
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
    pub(crate) fn with_file_btree(
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
    pub(crate) fn connection_state(&self) -> Arc<SharedState> {
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

/// A server running on a thread of its own ([`Server::spawn`]), stopped when this drops.
///
/// Stopping ends the accept loop, then shuts down the socket of every connection it
/// accepted, so each connection's thread reads the end of its stream and exits — it is not
/// joined: one waiting for the writer wakes at its `lock_timeout`, not at the shutdown.
/// The store is freed, and a persistent one's data-dir lock released, when the last
/// connection's thread lets go of it; until then [`Server::open`] of the same directory
/// is refused.
#[derive(Debug)]
pub struct ServerHandle {
    local_addr: SocketAddr,
    stop: Arc<Stop>,
    accept: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// The address the server listens on — under [`Server::spawn_loopback`], the port the
    /// kernel picked.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stop the server now; dropping the handle does the same.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(accept) = self.accept.take() else {
            return;
        };
        self.stop.requested.store(true, Ordering::SeqCst);
        // The accept loop is blocked in `accept`: a connection of its own wakes it to read
        // the flag. A listener on an unspecified address is reached on loopback.
        let mut wake = self.local_addr;
        if wake.ip().is_unspecified() {
            wake.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
        }
        if TcpStream::connect(wake).is_ok() {
            // Every stream the loop accepted is registered by the time it exits.
            drop(accept.join());
        } else {
            eprintln!("pglet: cannot wake the accept loop at {wake}; it is left running");
        }
        for (_, stream) in self.stop.connections.lock().drain() {
            drop(stream.shutdown(Shutdown::Both));
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A spawned server's stop request, and a handle on each connection's socket to close
/// when it stops, keyed by the connection's process id.
#[derive(Debug, Default)]
struct Stop {
    requested: AtomicBool,
    connections: Mutex<HashMap<i32, TcpStream>>,
}

/// A connection's entry in its spawned server's registry, removed when the connection's
/// thread ends, however it ends.
struct Registration {
    stop: Arc<Stop>,
    pid: i32,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.stop.connections.lock().remove(&self.pid);
    }
}

/// Process ids handed to connections, one each.
static NEXT_PID: AtomicI32 = AtomicI32::new(1);

/// Accept connections on `listener` for as long as it yields them — or, given a `stop`,
/// until it is requested — each on its own thread and served from
/// [`Server::connection_state`].
fn serve(listener: &TcpListener, server: &Server, stop: Option<&Arc<Stop>>) {
    for stream in listener.incoming() {
        if stop.is_some_and(|stop| stop.requested.load(Ordering::SeqCst)) {
            return;
        }
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
                // A spawned server closes this socket when it stops, so it keeps a handle
                // on it — registered here, on the accept thread, so every stream accepted
                // before the stop is in the registry once that thread has exited.
                // The entry is removed when its `Registration` drops — at the thread's
                // end, however it ends (a panic included, so the duplicate never keeps a
                // dead connection's socket open), or with the closure if no thread could
                // be spawned for it.
                let registration = match stop.map(|stop| stream.try_clone().map(|s| (stop, s))) {
                    None => None,
                    Some(Ok((stop, handle))) => {
                        stop.connections.lock().insert(pid, handle);
                        Some(Registration {
                            stop: Arc::clone(stop),
                            pid,
                        })
                    }
                    Some(Err(e)) => {
                        eprintln!("connection error: cannot keep a handle on the socket: {e}");
                        continue;
                    }
                };
                let state = server.connection_state();
                // The thread's share of the data-dir lock: the store this connection
                // writes is the locked directory's until the thread ends.
                let data_dir_lock = server.data_dir_lock.clone();
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
                        let served = connection::handle_connection(stream, pid, state);
                        // A stop's shutdown ends the connection with an error that is not
                        // the connection's own.
                        let stopping = registration
                            .as_ref()
                            .is_some_and(|r| r.stop.requested.load(Ordering::SeqCst));
                        if let Err(e) = served
                            && !stopping
                        {
                            eprintln!("connection error: {e}");
                        }
                        drop(registration);
                        // Released last, after the connection's store is no longer
                        // written by this thread.
                        drop(data_dir_lock);
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
    //! held across the reopen.

    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use pglet_core::file_lock::FileLock;
    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::{ScratchDir, open_database};

    use super::{Reopen, Server, ServerOptions, StorageKind, serve};
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
        std::thread::spawn(move || serve(&listener, &server, None));
        addr
    }

    /// After a durable write fails, the server returns to service with no operator, as
    /// 18.6's postmaster does after a `PANIC`: the connection that met the failure and
    /// an idle one in a block are each ended (`FATAL 57P02`) — the idle one after the
    /// reopen, so a connection is not served from a state it did not start on — and a
    /// reconnect reads exactly the durable rows and writes durably. The data-dir lock is
    /// held across the reopen.
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

    /// A spawned server serves on the loopback port it reports, and dropping its handle
    /// stops it: the listener is closed, and a connection still open reads the end of its
    /// stream instead of being served from a server no one holds.
    #[test]
    fn a_spawned_server_stops_when_its_handle_drops() {
        let handle = Server::open(&ServerOptions::default())
            .expect("open")
            .spawn_loopback()
            .expect("spawn");
        let addr = handle.local_addr();
        assert!(addr.ip().is_loopback(), "{addr}");
        assert_ne!(addr.port(), 0, "the port the kernel picked");

        let mut a = Client::connect(addr).expect("startup");
        a.query("CREATE TABLE t (id INT)").unwrap();
        a.query("INSERT INTO t VALUES (1)").unwrap();
        let mut b = Client::connect(addr).expect("a second connection");
        assert_eq!(
            ids(&mut b),
            Ok(vec!["1".to_string()]),
            "one store, two connections"
        );

        drop(handle);
        assert!(a.is_closed(), "an open connection is closed by the stop");
        assert!(b.is_closed(), "every open connection is");
        assert!(
            TcpStream::connect(addr).is_err(),
            "the listener is closed once the handle drops"
        );
    }

    /// The data-dir lock outlives the handle for as long as a connection can still write
    /// the store: a statement running when a spawned server's handle drops keeps another
    /// `Server::open` of the same directory out until it ends. Released with the handle,
    /// a second server opened over the directory while the first's statement still ran,
    /// and the old statement's commit then overwrote a write the second had acknowledged.
    #[test]
    fn the_data_dir_lock_outlives_the_handle_while_a_statement_runs() {
        let scratch = ScratchDir::new("server_lock_outlives_handle");
        let options = ServerOptions {
            data_dir: Some(scratch.path.clone()),
            ..ServerOptions::default()
        };
        let handle = Server::open(&options)
            .expect("open")
            .spawn_loopback()
            .expect("spawn");
        let mut a = Client::connect(handle.local_addr()).expect("startup");
        a.query("CREATE TABLE t (a INT)").unwrap();
        // Sized by profile to run for seconds, well past the drop below, in either build.
        let copies = if cfg!(debug_assertions) { 2 } else { 20 };
        let writer = std::thread::spawn(move || {
            let _ = a.query(&format!(
                "INSERT INTO t SELECT a FROM generate_series(1, 200) a, \
                 generate_series(1, 200) b, generate_series(1, {copies}) c"
            ));
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            !writer.is_finished(),
            "the statement must still be running when the handle drops"
        );
        drop(handle);
        assert!(
            Server::open(&options).is_err(),
            "the directory is still locked while the statement runs"
        );
        writer.join().expect("the client thread");
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let reopened = loop {
            if let Ok(server) = Server::open(&options) {
                break server;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the lock is released once the statement ends"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        drop(reopened);
    }

    /// Two spawned servers are two stores: each one's process-wide state is its own.
    #[test]
    fn two_spawned_servers_share_nothing() {
        let open = || {
            Server::open(&ServerOptions::default())
                .expect("open")
                .spawn_loopback()
                .expect("spawn")
        };
        let (one, two) = (open(), open());
        let mut a = Client::connect(one.local_addr()).expect("startup");
        a.query("CREATE TABLE t (id INT)").unwrap();
        let mut b = Client::connect(two.local_addr()).expect("startup");
        assert_eq!(ids(&mut b).map_err(|(code, _)| code), Err("42P01".into()));
        one.shutdown();
        b.query("CREATE TABLE t (id INT)").unwrap();
        assert_eq!(ids(&mut b), Ok(vec![]), "the other server is still serving");
    }

    /// `MemoryStorage` keeps nothing, so it refuses a data dir, as `--storage memory
    /// --data-dir` is refused.
    #[test]
    fn memory_storage_refuses_a_data_dir() {
        let options = ServerOptions {
            storage: StorageKind::Memory,
            data_dir: Some("/nonexistent".into()),
            ..ServerOptions::default()
        };
        let err = Server::open(&options).expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }
}
