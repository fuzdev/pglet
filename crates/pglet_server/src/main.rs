//! `pglet_server` — the native TCP server: a thread per connection over the `pglet` embed,
//! speaking the PostgreSQL wire protocol (`pglet_wire`). Listens on `127.0.0.1` only, with no
//! authentication; `--help` lists the flags, and `dump` exports a persisted database as SQL.

mod cli;
#[cfg(test)]
mod clock_tests;
mod connection;
mod copy;
mod encoding;
mod execute;
mod handlers;
mod server;
mod startup;
#[cfg(test)]
mod test_support;
mod writer;

use std::net::TcpListener;

use cli::StorageKind;
use connection::SharedState;
use server::{Server, on_connection_stack};

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

    // `dump` subcommand: open the persisted database and print its portable SQL dump
    // to stdout, then exit — no listener. The operational "export a DB" path; the
    // dump's target is loading into both stock Postgres and pglet, the dump invariant
    // `docs/sequences.md` states (see `pglet::Database::dump_sql`).
    if args.dump {
        let Some(dir) = args.data_dir else {
            return Err("dump requires --data-dir <PATH>".into()); // parse_args refuses it first
        };
        // `open_existing` (not `open`): a mistyped `--data-dir` must fail loud, not
        // silently create an empty store and print a near-empty dump.
        let dump = on_connection_stack(move || {
            let db = pglet::Database::open_existing(&dir).map_err(|e| e.to_string())?;
            db.dump_sql().map_err(|e| e.to_string())
        })??;
        print!("{dump}");
        return Ok(());
    }

    let port = args.port.unwrap_or(5433);
    let coerce_unknown_types = args.coerce_unknown_types;
    let lock_timeout_ms = args
        .lock_timeout_ms
        .unwrap_or(pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS);

    // The server: the state connections are served from (storage + schema), and — for a
    // persistent store — how to reopen it once it fails a durable write (`server`).
    // `parse_args` has refused `--storage memory` with a `--data-dir`.
    let server = match (args.storage, &args.data_dir) {
        (Some(StorageKind::Memory), _) => {
            println!("pglet using in-memory storage (no persistence)");
            Server::new(
                SharedState::in_memory()
                    .with_coerce_unknown_types(coerce_unknown_types)
                    .with_default_lock_timeout(lock_timeout_ms),
                None,
                None,
            )
        }
        (_, Some(dir)) => {
            println!(
                "pglet using B+tree storage with persistence at {}",
                dir.display()
            );
            Server::with_file_btree(dir, coerce_unknown_types, lock_timeout_ms)?
        }
        (Some(StorageKind::Btree) | None, None) => {
            println!("pglet using B+tree storage (in-memory, no persistence)");
            Server::new(
                SharedState::with_btree()
                    .with_coerce_unknown_types(coerce_unknown_types)
                    .with_default_lock_timeout(lock_timeout_ms),
                None,
                None,
            )
        }
    };
    if coerce_unknown_types {
        println!("pglet coercing unsupported types to TEXT (--coerce-unknown-types)");
    }

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)?;
    // The bound address, not the requested one: under `--port 0` the kernel picks the
    // port, and this line is where a harness reads it.
    println!("pglet listening on {}", listener.local_addr()?);

    server::serve(&listener, &server);

    Ok(())
}
