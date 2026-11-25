//! `--exit-on-stdin-close`: the server ends when its standard input does, and not before.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A ceiling for a loaded box, not an expectation.
const LIMIT: Duration = Duration::from_secs(30);

/// Kill and reap the child however the test leaves.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn the_server_exits_when_its_stdin_closes() {
    let mut child = Reaped(
        Command::new(env!("CARGO_BIN_EXE_pglet_server"))
            .args(["--port", "0", "--exit-on-stdin-close"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn pglet_server"),
    );
    // The listening line is read on a thread that then ends, closing the read end of the
    // server's stdout: a parent that dies takes that pipe with it, so the exit must not
    // write there.
    let out = child.0.stdout.take().expect("stdout is piped");
    let (tx, ports) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if let Some(port) = line.strip_prefix("pglet listening on 127.0.0.1:") {
                let _ = tx.send(port.parse::<u16>().ok());
                break;
            }
        }
    });
    let port = ports
        .recv_timeout(LIMIT)
        .expect("the server reports its port in time")
        .expect("a port");
    reader
        .join()
        .expect("the reader ends, closing stdout's read end");
    // Held open, stdin keeps the server serving.
    std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    assert!(
        child.0.try_wait().expect("try_wait").is_none(),
        "the server runs while its stdin is open"
    );

    drop(child.0.stdin.take());

    let deadline = Instant::now() + LIMIT;
    let status = loop {
        if let Some(status) = child.0.try_wait().expect("try_wait") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the server exits once stdin closes"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status}");
}
