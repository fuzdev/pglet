//! A connection the server cannot spawn a thread for is refused, and the server keeps
//! serving — driven through a real `pglet_server` whose address space is capped.
//!
//! Each connection thread reserves `SERVER_THREAD_STACK` of address space, so under an
//! address-space limit (`ulimit -v`) the spawn fails once enough connections are held.
//! That failure used to propagate out of the accept loop and **exit the process**. The
//! refused connection gets 18.6's own answer when it cannot fork a backend — one
//! protocol-2 error packet — and every other connection, held or new, is unaffected.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How long a server gets to report that it is listening: a ceiling for a loaded box,
/// not an expectation.
const START_LIMIT: Duration = Duration::from_secs(30);

/// The most connections the test holds before it expects one refused.
const MAX_HELD: usize = 40;

/// A spawned in-memory server, killed and reaped however the test leaves.
struct Server {
    child: Child,
    port: u16,
}

impl Server {
    /// Spawn the server with its address space capped at `limit_kib` (unlimited when
    /// `None`). One malloc arena, so the cap is met by thread stacks rather than by
    /// arena reservations. The port is the kernel's pick (`--port 0`), read from the
    /// server's own `pglet listening on …` line — a port picked in advance and probed by
    /// connecting can be taken between the pick and the bind, and whatever answered on
    /// it passed for the server.
    fn spawn(limit_kib: Option<u64>) -> Self {
        let limit = limit_kib.map_or_else(|| "unlimited".to_owned(), |k| k.to_string());
        let mut child = Command::new("sh")
            .args([
                "-c",
                &format!("ulimit -v {limit} && exec \"$0\" --port 0"),
                env!("CARGO_BIN_EXE_pglet_server"),
            ])
            .env("MALLOC_ARENA_MAX", "1")
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

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// The process's address-space size, from `/proc`.
    fn vm_size_kib(&self) -> u64 {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.child.id()))
            .expect("read /proc status");
        status
            .lines()
            .find_map(|l| l.strip_prefix("VmSize:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmSize")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a startup handshake got back.
enum Startup {
    /// `ReadyForQuery`: the connection is being served.
    Ready(TcpStream),
    /// Everything the server sent before closing, when it was not a served connection.
    Refused(Vec<u8>),
}

fn startup(port: u16) -> Startup {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
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
    // A refused connection may already be closed; what it sent is still readable.
    let _ = stream.write_all(&msg);
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if got.windows(6).any(|w| w == b"Z\0\0\0\x05I") {
            return Startup::Ready(stream);
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return Startup::Refused(got),
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
}

/// `SELECT 1` on a served connection answers `1`.
fn select_one(stream: &mut TcpStream) -> bool {
    let sql = b"SELECT 1\0";
    let mut msg = vec![b'Q'];
    msg.extend(u32::try_from(sql.len() + 4).unwrap().to_be_bytes());
    msg.extend_from_slice(sql);
    stream.write_all(&msg).unwrap();
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while !got.windows(5).any(|w| w == b"Z\0\0\0\x05") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(n) => got.extend_from_slice(&buf[..n]),
        }
    }
    // One `DataRow` of one column holding `1`.
    let row: &[u8] = b"D\0\0\0\x0b\0\x01\0\0\0\x011";
    got.windows(row.len()).any(|w| w == row)
}

#[test]
fn a_connection_without_a_thread_is_refused_and_the_server_keeps_serving() {
    // The server's own footprint, measured unlimited, plus room for two and a half
    // connection threads: the third or so spawn fails, whatever this build's
    // reservation is.
    let base = {
        let server = Server::spawn(None);
        server.vm_size_kib()
    };
    let stack_kib = (pglet_sql::depth::SERVER_THREAD_STACK / 1024) as u64;
    let mut server = Server::spawn(Some(base + stack_kib * 5 / 2));

    let mut held = Vec::new();
    let refusal = loop {
        assert!(held.len() < MAX_HELD, "no connection was refused");
        match startup(server.port) {
            Startup::Ready(stream) => held.push(stream),
            Startup::Refused(bytes) => break bytes,
        }
    };
    // The server is alive and says why, in 18.6's words and framing.
    assert!(server.alive(), "the server exited on a failed spawn");
    let text = String::from_utf8_lossy(&refusal);
    assert!(
        text.starts_with("Ecould not fork new process for connection: ") && text.ends_with("\n\0"),
        "{text:?}"
    );
    assert!(!held.is_empty(), "the cap refused the first connection");

    // A held connection is still served, and once they close a new one is too.
    assert!(
        select_one(&mut held[0]),
        "a held connection stopped answering"
    );
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut fresh = loop {
        match startup(server.port) {
            Startup::Ready(stream) => break stream,
            Startup::Refused(_) => {
                assert!(
                    Instant::now() < deadline,
                    "no connection served after release"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    assert!(select_one(&mut fresh));
    assert!(server.alive());
}
