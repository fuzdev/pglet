//! `pglet_server` — the native TCP server: a thread per connection over the `pglet` embed,
//! speaking the PostgreSQL wire protocol (`pglet_wire`), with no authentication.
//!
//! The binary (`main.rs`) is a thin caller of this library, which is also how a Rust test
//! suite runs the server in its own process: [`Server::open`] builds the state from
//! [`ServerOptions`], and [`Server::spawn_loopback`] (or [`Server::spawn`] over a caller's
//! listener) serves it from a thread of its own until the returned [`ServerHandle`] is
//! dropped.
//!
//! ```no_run
//! let server = pglet_server::Server::open(&pglet_server::ServerOptions::default())?;
//! let handle = server.spawn_loopback()?;
//! let url = format!("postgres://pglet@{}/pglet", handle.local_addr());
//! # drop(url);
//! drop(handle); // stops accepting and closes every connection's socket
//! # Ok::<(), std::io::Error>(())
//! ```

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

pub use server::{Server, ServerHandle, ServerOptions, StorageKind, dump_sql};
