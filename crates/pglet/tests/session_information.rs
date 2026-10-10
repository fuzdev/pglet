//! The session-information forms on the **embed** host — the leg no differential
//! fixture can reach.
//!
//! The `tests/fixtures/session/*` corpus compares every one of these against a live
//! 18.6, but only over the **native server**, and only for the values a startup
//! packet supplies. Two things are outside it and are the whole point of this file:
//!
//! - **What a host with no startup packet answers.** `pglet::Database` (and the wasm
//!   bindings over it) carry no `user` and no `database`, so `current_user` and
//!   `current_database()` answer `pglet` rather than echoing a client. Before the
//!   session forms existed, every spelling here was `42703` on this host and `0A000`
//!   for the call forms, and a host-shaped regression would put one of those back
//!   with the whole fixture corpus still green.
//! - **That evaluation reaches them at all off the socket.** These read the *ambient*
//!   session (`SessionSettings::install_ambient`, whose doc enumerates every install
//!   site in one place), installed per statement by
//!   `catalog::execute_session_command` and — for this facade's own COPY entry points
//!   — by `Database::copy_in` / `copy_out`. Nothing in the fixture harness exercises
//!   either, because the native server installs at sites of its own.

// A test harness: `.unwrap()`/`.expect()`/`panic!` surface a setup failure — or a
// statement that unexpectedly succeeded — loudly right here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_sql::catalog::SessionSettings;
use pglet_sql::types::{TypeOid, Value};

/// Read a single scalar cell from a one-statement query.
fn scalar(db: &mut Database, sql: &str) -> Value {
    let mut results = db.execute(sql).expect("query");
    results.remove(0).rows[0][0].clone()
}

/// The declared `TypeOid` of a one-column query's only output column.
fn declared(db: &mut Database, sql: &str) -> TypeOid {
    let mut results = db.execute(sql).expect("query");
    results.remove(0).columns()[0].type_oid
}

/// The SQLSTATE a statement fails with, or a panic naming what it returned instead.
fn sqlstate(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(_) => panic!("{sql} unexpectedly succeeded"),
        Err(e) => e.sqlstate().to_string(),
    }
}

/// The role and database a packet-less host reports.
///
/// `pglet` and not `postgres`: a fabricated cluster identity would read as a fact
/// about a catalog this engine does not have.
#[test]
fn a_packetless_host_answers_its_own_identity() {
    let mut db = Database::with_btree();
    for sql in [
        "SELECT current_user",
        "SELECT session_user",
        "SELECT user",
        "SELECT current_role",
        "SELECT current_catalog",
        "SELECT current_database()",
    ] {
        assert_eq!(
            scalar(&mut db, sql),
            Value::Text("pglet".into()),
            "{sql} on the embed host"
        );
    }
}

/// The schema and search-path readers are constants on every host, so the embed leg
/// answers exactly what the fixtures assert over the socket.
#[test]
fn the_search_path_readers_agree_with_the_server_leg() {
    let mut db = Database::with_btree();
    assert_eq!(
        scalar(&mut db, "SELECT current_schema"),
        Value::Text("public".into())
    );
    assert_eq!(
        scalar(&mut db, "SELECT current_schema()"),
        Value::Text("public".into())
    );
    assert_eq!(
        scalar(&mut db, "SELECT current_setting('search_path')"),
        Value::Text("\"$user\", public".into())
    );
    let Value::Array(schemas) = scalar(&mut db, "SELECT current_schemas(true)") else {
        panic!("current_schemas must answer an array")
    };
    assert_eq!(schemas.element_type, TypeOid::Name);
    assert_eq!(
        schemas.elements,
        vec![
            Value::Text("pg_catalog".into()),
            Value::Text("public".into())
        ]
    );
}

/// `version()`'s banner carries the release every version sniff reads, and it is the
/// **same** release `SHOW server_version` reports — the one fact, two spellings.
#[test]
fn the_version_banner_and_show_server_version_name_one_release() {
    let mut db = Database::with_btree();
    let Value::Text(banner) = scalar(&mut db, "SELECT version()") else {
        panic!("version() must answer text")
    };
    let Value::Text(reported) = scalar(&mut db, "SHOW server_version") else {
        panic!("SHOW server_version must answer text")
    };
    assert!(
        banner.starts_with(&format!("PostgreSQL {reported} (")),
        "version() banner {banner:?} does not open with the reported release {reported:?}"
    );
    assert!(
        banner.contains(" on "),
        "version() banner {banner:?} drops the platform clause a loose sniff splits on"
    );
}

/// The declared types on this host, which is the half `Database` shares with the
/// wire: `name` (19) for the identity forms, `name[]` (1003) for `current_schemas`,
/// `text` for `version()` and `current_setting`.
#[test]
fn the_declared_types_are_name_and_not_text() {
    let mut db = Database::with_btree();
    for sql in [
        "SELECT current_user",
        "SELECT current_catalog",
        "SELECT current_database()",
        "SELECT current_schema",
        "SELECT current_schema()",
    ] {
        assert_eq!(declared(&mut db, sql), TypeOid::Name, "{sql}");
    }
    assert_eq!(
        declared(&mut db, "SELECT current_schemas(false)"),
        TypeOid::NameArray
    );
    assert_eq!(declared(&mut db, "SELECT version()"), TypeOid::Text);
    assert_eq!(
        declared(&mut db, "SELECT current_setting('search_path')"),
        TypeOid::Text
    );
}

/// The refusals, on the host whose parse-time and eval-time paths the fixture
/// harness never drives: a call spelling of a keyword form, every wrong arity, the
/// bare non-keyword names, and an unrecognized parameter.
#[test]
fn the_refusals_carry_the_same_sqlstates_off_the_socket() {
    let mut db = Database::with_btree();
    for (sql, code) in [
        ("SELECT current_user()", "42601"),
        ("SELECT session_user()", "42601"),
        ("SELECT current_catalog()", "42601"),
        ("SELECT current_schema(1)", "42883"),
        ("SELECT current_database(1)", "42883"),
        ("SELECT version(1)", "42883"),
        ("SELECT current_setting()", "42883"),
        ("SELECT current_setting('a', true, false)", "42883"),
        ("SELECT current_schemas()", "42883"),
        ("SELECT current_schemas", "42703"),
        ("SELECT current_database", "42703"),
        ("SELECT version", "42703"),
        ("SELECT current_setting('nosuchthing')", "42704"),
    ] {
        assert_eq!(sqlstate(&mut db, sql), code, "{sql}");
    }
    assert_eq!(
        scalar(&mut db, "SELECT current_setting('nosuchthing', true)"),
        Value::Null,
        "missing_ok turns the 42704 into NULL"
    );
}

/// The **embed/wasm** half of the ambient-session install, and the one thing on this
/// host that a stale ambient copy can be seen through.
///
/// The identity is a constant here, so a missing install answers the same `pglet`
/// either way — `application_name` is the session value that *moves*, and
/// `current_setting` reads it through the ambient session while `SHOW` reads the
/// caller's own. Measured against a build with
/// `catalog::execute_session_command`'s `install_ambient()` deleted:
/// `current_setting('application_name')` answered `''` after a `SET … TO 'probe'`
/// that `SHOW application_name` reported correctly — one connection contradicting
/// itself about a value, with every fixture green.
#[test]
fn a_set_moves_what_current_setting_reads() {
    let mut db = Database::with_btree();
    assert_eq!(
        scalar(&mut db, "SELECT current_setting('application_name')"),
        Value::Text(String::new())
    );
    db.execute("SET application_name TO 'probe'").unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT current_setting('application_name')"),
        Value::Text("probe".into()),
        "current_setting must read the session the SET moved"
    );
    // ...and it is the same value `SHOW` reports, which is the point of reading one
    // table rather than two.
    assert_eq!(
        scalar(&mut db, "SHOW application_name"),
        Value::Text("probe".into())
    );
    db.execute("RESET application_name").unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT current_setting('application_name')"),
        Value::Text(String::new())
    );
}

/// `copy_in` installs the ambient session too — the embed facade's own COPY entry
/// point, which bypasses `execute_as`/`execute_bound` and so
/// `catalog::execute_session_command`'s install.
///
/// **No ordinary statement sequence can see this**, which is why the foreign session
/// is installed by hand: every embed/wasm host answers the one
/// `DEFAULT_SESSION_NAME` constant (the facade exposes no way to set an identity —
/// `SessionSettings::with_identity` exists and only the native server calls it), so
/// a missed install reads back the same `pglet` either way. What the hand-install
/// stands in for is the shape that *is* reachable: two sessions on one thread, which
/// the native server has as a live wrong-row defect over its own COPY routes
/// (`pglet_server::copy`'s `ambient_session_tests`). The day `Database` takes an
/// identity, this is that defect here with no other code change.
#[test]
fn copy_in_installs_the_ambient_session() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE cp_owned (i INT, u TEXT DEFAULT current_user)")
        .unwrap();
    // A foreign session, as a second `Database` or a server connection on this thread
    // would leave behind.
    SessionSettings::new(String::new())
        .with_identity("bob".into(), "elsewhere".into())
        .install_ambient();
    db.copy_in("COPY cp_owned (i) FROM STDIN", b"1\n").unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT u FROM cp_owned"),
        Value::Text("pglet".into()),
        "copy_in must resolve the stored DEFAULT against its own session"
    );
}

/// A stored `DEFAULT` resolves through the ambient session at **insert** time, not
/// at `CREATE TABLE` time — the property that makes the forms ordinary persisted
/// expressions rather than folded constants, and the one the fixture corpus cannot
/// separate from the value because the server's identity never moves mid-session.
#[test]
fn a_stored_default_resolves_per_insert() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE owned (id INT, who TEXT DEFAULT current_user)")
        .unwrap();
    db.execute("INSERT INTO owned (id) VALUES (1)").unwrap();
    assert_eq!(
        scalar(&mut db, "SELECT who FROM owned WHERE id = 1"),
        Value::Text("pglet".into())
    );
    // ...and the catalog stored the expression rather than the value: the rendered
    // default is the keyword, which is also what the dump emits.
    assert_eq!(
        scalar(
            &mut db,
            "SELECT column_default FROM information_schema.columns \
             WHERE table_name = 'owned' AND column_name = 'who'"
        ),
        Value::Text("CURRENT_USER".into())
    );
}
