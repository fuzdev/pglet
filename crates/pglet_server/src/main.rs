//! `pglet_server` — the native TCP server: a thread per connection over the `pglet` embed,
//! speaking the PostgreSQL wire protocol (`pglet_wire`). Listens on `127.0.0.1` only, with no
//! authentication; `--help` lists the flags, and `dump` exports a persisted database as SQL.

mod cli;

use std::net::{Ipv4Addr, TcpListener};

use pglet_server::{Server, ServerOptions, StorageKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match cli::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("pglet_server: {message}");
            std::process::exit(2);
        }
    };
    if args.help {
        println!("pglet_server - a small Postgres-compatible-ish database server");
        println!();
        println!("Usage: pglet_server [OPTIONS]");
        println!("       pglet_server dump --data-dir <PATH>");
        println!();
        println!("Options:");
        println!("  --port <PORT>        Port to listen on (default: 5433; 0 lets the kernel");
        println!("                       pick one, reported on the listening line)");
        println!("  --storage <TYPE>     Storage backend: btree (default) or memory");
        println!("                       - btree: CoW B+tree with MVCC; in memory unless");
        println!("                         --data-dir makes it persistent");
        println!("                       - memory: no MVCC and no persistence; a failed");
        println!("                         autocommit statement's partial effects stand;");
        println!("                         refuses --data-dir");
        println!("  --data-dir <PATH>    Persist the B+tree to this directory");
        println!("  --coerce-unknown-types  Coerce a type name pglet does not model to TEXT");
        println!("                       instead of erroring (default: error)");
        println!("  --lock-timeout <VALUE>  How long a write waits for the writer (the one");
        println!("                       connection whose transaction block may write)");
        println!("                       before 55P03, as a lock_timeout value: 1000, 1s,");
        println!("                       500ms; 0 waits without a bound (default: 1s)");
        println!("  --exit-on-stdin-close  Exit (status 0) once standard input reaches its");
        println!("                       end, so a parent that dies, killed or not, takes");
        println!("                       the server with it; the parent holds a pipe open");
        println!("                       for the server's life (an empty or closed stdin");
        println!("                       exits at once)");
        println!("  --help               Show this help message");
        println!();
        println!("An unknown argument, a repeated flag or an unparsable value is an error.");
        println!();
        println!("The server listens on 127.0.0.1 only and accepts any credentials: run it");
        println!("on a trusted machine. Each connection thread reserves 64 MiB of address");
        println!("space (512 MiB in a debug build); a connection no thread can be spawned");
        println!("for is refused.");
        println!();
        println!("Subcommands:");
        println!("  dump --data-dir <PATH>  Print a portable SQL dump of the persisted");
        println!("                       database to stdout and exit (no server). It reads");
        println!("                       the store at a connection thread's depth limit, so");
        println!("                       it exports a store an embedded opener refuses (54001)");
        return Ok(());
    }

    // `dump` subcommand: print the persisted database's portable SQL dump to stdout, then
    // exit — no listener. The operational "export a DB" path; the dump's target is loading
    // into both stock Postgres and pglet, the dump invariant `docs/sequences.md` states.
    if args.dump {
        let Some(dir) = args.data_dir else {
            return Err("dump requires --data-dir <PATH>".into()); // parse_args refuses it first
        };
        // The message alone, as a `String`, so `main`'s report prints it as it always
        // has rather than the `io::Error`'s `Debug` form.
        print!(
            "{}",
            pglet_server::dump_sql(&dir).map_err(|e| e.to_string())?
        );
        return Ok(());
    }

    // `parse_args` has refused `--storage memory` with a `--data-dir`.
    let options = ServerOptions {
        storage: args.storage.unwrap_or_default(),
        data_dir: args.data_dir,
        coerce_unknown_types: args.coerce_unknown_types,
        lock_timeout_ms: args
            .lock_timeout_ms
            .unwrap_or(pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS),
    };
    match (options.storage, &options.data_dir) {
        (StorageKind::Memory, _) => println!("pglet using in-memory storage (no persistence)"),
        (StorageKind::Btree, Some(dir)) => println!(
            "pglet using B+tree storage with persistence at {}",
            dir.display()
        ),
        (StorageKind::Btree, None) => {
            println!("pglet using B+tree storage (in-memory, no persistence)");
        }
    }
    // The state connections are served from (storage + schema), and — for a persistent
    // store — how to reopen it once it fails a durable write.
    let server = Server::open(&options)?;
    if options.coerce_unknown_types {
        println!("pglet coercing unsupported types to TEXT (--coerce-unknown-types)");
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, args.port.unwrap_or(5433)))?;
    // The bound address, not the requested one: under `--port 0` the kernel picks the
    // port, and this line is where a harness reads it.
    println!("pglet listening on {}", listener.local_addr()?);

    if args.exit_on_stdin_close {
        exit_on_stdin_close();
    }

    server.serve(&listener);

    Ok(())
}

/// Watch standard input on a thread of its own and end the process once it reaches its
/// end — `--exit-on-stdin-close`, the test-runner idiom: a parent holds the write end of
/// a pipe for the server's life, and the kernel closes it however the parent ends, so a
/// killed runner leaves no server behind. The exit is immediate, as a `kill` is: a
/// statement in flight is lost, an open block rolls back with the process, and a
/// persistent store keeps its last commit (the double-meta commit point).
fn exit_on_stdin_close() {
    use std::io::Read;
    std::thread::spawn(|| {
        let mut stdin = std::io::stdin().lock();
        let mut sink = [0_u8; 4096];
        loop {
            match stdin.read(&mut sink) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                // An unreadable stdin can no more say the parent is alive than a closed one.
                Err(_) => break,
            }
        }
        // Nothing is written on the way out: the parent this watches is gone, and a pipe
        // it read the server's output through is gone with it, where a write would fail.
        std::process::exit(0);
    });
}
