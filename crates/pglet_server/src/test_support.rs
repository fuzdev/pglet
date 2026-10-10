//! Helpers shared by this crate's test modules.
//!
//! They were written once per test module — twelve copies of five functions across
//! four files, all byte-identical, and two of them under a second name (`tags` /
//! `command_tags`, `sqlstate` / `error_code`). Nothing about a test module makes
//! its message-inspection helpers its own, so a new module imports from here
//! rather than starting a thirteenth copy.
//!
//! It lives at the crate root rather than beside one of its callers because a
//! private `mod` inside `execute.rs` is what left the other three files with
//! their copies — the first attempt at this, which deduplicated one file and read
//! as finished.

use std::sync::Arc;

use pglet_wire::BackendMessage;

use crate::connection::{Connection, SharedState};
use crate::execute::{Protocol, execute_query_core};

/// A fresh connection over an in-memory CoW-btree engine (no socket).
pub fn conn() -> Connection {
    Connection::new(1, Arc::new(SharedState::with_btree()))
}

/// Drive `sql` through the simple-query path and return the reply frames.
pub fn run(conn: &mut Connection, sql: &str) -> Vec<BackendMessage> {
    execute_query_core(conn, sql, &Protocol::Simple, &[], None)
}

/// Every `CommandComplete` tag, in order.
pub fn tags(msgs: &[BackendMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            BackendMessage::CommandComplete { tag } => Some(tag.clone()),
            _ => None,
        })
        .collect()
}

/// The SQLSTATE of the first `ErrorResponse`, if any.
pub fn sqlstate(msgs: &[BackendMessage]) -> Option<String> {
    msgs.iter().find_map(|m| match m {
        BackendMessage::ErrorResponse { fields } => fields
            .iter()
            .find(|(k, _)| *k == b'C')
            .map(|(_, v)| v.clone()),
        _ => None,
    })
}

/// The first column of every row `msgs` carries, as text (`NULL` for a null).
pub fn cells(msgs: &[BackendMessage]) -> Vec<String> {
    msgs.iter()
        .filter_map(|m| match m {
            BackendMessage::DataRow { values } => {
                Some(values.first().cloned().flatten().map_or_else(
                    || "NULL".to_string(),
                    |v| String::from_utf8_lossy(&v).into_owned(),
                ))
            }
            _ => None,
        })
        .collect()
}

/// Whether any frame is an `ErrorResponse`. Prefer [`sqlstate`] where the code
/// matters — an assertion on the code fails with the *wrong* code in the message,
/// where this one fails with `false`.
pub fn has_error(msgs: &[BackendMessage]) -> bool {
    msgs.iter()
        .any(|m| matches!(m, BackendMessage::ErrorResponse { .. }))
}

/// A `PreparedStatement` standing in for one a real `Parse` would build, for a
/// test whose subject is the *namespace* rather than the statement.
pub fn prepared_stub() -> crate::connection::PreparedStatement {
    crate::connection::PreparedStatement {
        query: "SELECT 1".into(),
        param_types: Vec::new(),
        param_enums: Vec::new(),
        declared_param_types: Vec::new(),
        plan: None,
        plan_gen: 0,
    }
}
