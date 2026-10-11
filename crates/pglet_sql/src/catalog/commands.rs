//! The session-hygiene statements that run ahead of the planner — `SET`, `SHOW`,
//! `DEALLOCATE`, `DISCARD` and `RESET`.
//!
//! This is the seam a driver or a connection pool talks to without any user SQL
//! being involved: what it sends on connect and on check-in lands here. The rule
//! the whole module exists to keep is that each of those statements **answers what
//! PostgreSQL answers, or fails loud** — never a third thing quietly. `SET`'s
//! GUC-echo family is the accepted-no-op part of that (`SET application_name = …`
//! changes nothing pglet models and reporting `SET` is accurate), and
//! [`execute_set_command`]'s two refusals are the part that would otherwise lie.
//!
//! `SET`/`SHOW` are client configuration commands that don't touch the database.
//! `DEALLOCATE` addresses the session's **prepared-statement namespace**, which the
//! planner likewise cannot see: it lives on whatever a host calls a connection.
//! `DISCARD` reaches both that namespace and the session's sequence state;
//! `RESET` is the GUC family's other spelling.
//!
//! [`execute_session_command`] is the one dispatcher over all five, and every host
//! calls it — the native server per statement after `split_statements`, the embed
//! facade (and the wasm bindings over it) in `Database::execute_as` and
//! `execute_bound` for every statement it does not parse first (a session command
//! never parses). Written per host it drifted: `SET`/`SHOW` were serviced on the
//! server and `0A000` on the embed leg, so the same statement had two answers
//! depending on which one a consumer wired up.
//!
//! # Fold width
//!
//! The `SET`/`SHOW` pair classifies **ahead of the parser** on the raw text, so its
//! case fold is ASCII — the lexer's width — and so is the whitespace it skips and
//! splits on (`lexer::is_space`): a Unicode space is a name character to 18.6, so
//! `<U+00A0>SET …` is `42601` there. U+017F LATIN SMALL LETTER LONG S
//! uppercases to ASCII `S`: under a Unicode fold `ſET client_min_messages =
//! 'warning'` ran here as a `SET` where 18.6 answers `42601 syntax error at or near
//! "ſET"`, and `ſHOW search_path` answered `42704 unrecognized configuration
//! parameter "w search_path"` against the same `42601`. `parse_show_name` folds
//! ASCII too, one step further in: 18.6 reports an unknown parameter under the
//! scanner's ASCII downcase — `SHOW FÖÖ` is `"fÖÖ"` there, where a Unicode fold
//! reported `"föö"`.
//!
//! `DEALLOCATE`, `DISCARD` and `RESET` classify off the **lexer** rather than the
//! raw text, because each has to read a word after the statement keyword and fold
//! it as the grammar does. That is also why a leading comment does not defeat those
//! three where it defeats their two neighbours (`/* trace */ SET …` is `0A000`):
//! the lexer has already dropped comments. 18.6 accepts a leading comment on all
//! five, so the split is pglet's, and it is `EDGES.md`'s to carry, not this
//! module's to hide.
use std::borrow::Cow;
use std::collections::HashMap;

use crate::executor::{IsolationLevel, QueryResult, SequenceSession};
use crate::lexer::{
    Keyword, Token, TokenKind, all_tokens, is_space, is_word, lead_tokens, trim_space,
};
use crate::parser::lex_error_to_error;
use crate::types::{ColumnDesc, ResultShape, TypeOid, Value};
use crate::{Error, Result};

/// Checks if query is a SET command (client configuration).
pub fn is_set_command(sql: &str) -> bool {
    leads_with_word(sql, "SET")
}

/// Checks if query is a SHOW command.
pub fn is_show_command(sql: &str) -> bool {
    leads_with_word(sql, "SHOW")
}

/// Whether `sql`, less the lexer's whitespace at either end, starts with `word` under an
/// ASCII fold and then a byte of that whitespace — the classifier the `SET`/`SHOW` pair
/// runs ahead of the lexer, at the lexer's widths of both.
fn leads_with_word(sql: &str, word: &str) -> bool {
    let text = trim_space(sql).as_bytes();
    text.len() > word.len()
        && text[..word.len()].eq_ignore_ascii_case(word.as_bytes())
        && is_space(text[word.len()])
}

/// The one schema a pglet database has.
///
/// `search_path` is the seam where that shows: Postgres resolves an unqualified
/// name through the list and pglet has nowhere else to look, so a list naming
/// anything else cannot be honoured — see [`check_search_path_elements`].
const ONLY_SCHEMA: &str = "public";

/// Postgres's own `search_path` default, which pglet reports verbatim.
///
/// It is the true value for this engine and not a borrowed string: `$user` names
/// a schema called after the session user, which a fresh Postgres database does not
/// have either, so 18.6 resolves this list to `public` alone exactly as pglet does.
/// Measured on 18.6 — `SHOW search_path`, the startup `ParameterStatus`, and what a
/// `RESET search_path` / `DISCARD ALL` restores are all this string.
pub const DEFAULT_SEARCH_PATH: &str = "\"$user\", public";

/// `search_path` elements that select **no schema of their own** in a pglet
/// database, and are therefore honourable rather than refusable.
///
/// `$user` names a schema called after the session user, which pglet never has, and
/// `pg_catalog` is searched implicitly in Postgres whether or not the list mentions
/// it. Neither can route an unqualified name away from `public`, which is the only
/// property this gate is about.
const IGNORABLE_SEARCH_PATH_ELEMENTS: [&str; 2] = ["$user", "pg_catalog"];

/// The release pglet's dialect targets — the one home for the number, read by
/// `SHOW server_version`, by `server_version_num`, and by [`crate::builtins::version`].
///
/// `server_version_num`'s own literal is held to this one by a compile-time assertion.
///
/// It names the **dialect target**, not feature completeness: pglet implements a
/// subset of that release's SQL, and `docs/compatibility.md` says which.
pub const SERVER_VERSION: &str = "18.6";

/// [`SERVER_VERSION`] as the number a client feature-gates on — `server_version_num`'s
/// answer, `major * 10000 + minor`.
///
/// A literal because a `const` cannot format one, held to [`SERVER_VERSION`] at
/// compile time by [`version_num_spells`], so moving one without the other fails
/// the build.
const SERVER_VERSION_NUM: &str = "180006";

const _: () = assert!(
    version_num_spells(SERVER_VERSION, SERVER_VERSION_NUM),
    "SERVER_VERSION_NUM must be SERVER_VERSION's major * 10000 + minor"
);

/// Whether `num` is `version`'s `major.minor` as `major * 10000 + minor`, in decimal
/// with no leading zero — the relation 18.x's `server_version_num` holds to its
/// `server_version`. A `version` that is not two dot-separated digit runs is `false`.
const fn version_num_spells(version: &str, num: &str) -> bool {
    let v = version.as_bytes();
    let mut major: u64 = 0;
    let mut minor: u64 = 0;
    let mut i = 0;
    let mut seen_dot = false;
    let mut digits_before = 0;
    let mut digits_after = 0;
    while i < v.len() {
        let b = v[i];
        if b == b'.' {
            if seen_dot {
                return false;
            }
            seen_dot = true;
        } else if b.is_ascii_digit() {
            let d = (b - b'0') as u64;
            if seen_dot {
                minor = minor * 10 + d;
                digits_after += 1;
            } else {
                major = major * 10 + d;
                digits_before += 1;
            }
        } else {
            return false;
        }
        i += 1;
    }
    if !seen_dot || digits_before == 0 || digits_after == 0 {
        return false;
    }
    let want = major * 10000 + minor;
    let n = num.as_bytes();
    if n.is_empty() || n[0] == b'0' {
        return false;
    }
    let mut got: u64 = 0;
    let mut j = 0;
    while j < n.len() {
        if !n[j].is_ascii_digit() {
            return false;
        }
        got = got * 10 + (n[j] - b'0') as u64;
        j += 1;
    }
    got == want
}

/// The schema an unqualified name resolves to — what `current_schema` answers.
///
/// Derived from [`DEFAULT_SEARCH_PATH`] by the same reading
/// [`IGNORABLE_SEARCH_PATH_ELEMENTS`] states: `$user` names a schema pglet never
/// has, so the first element that selects a real schema is `public`. Because that
/// path never moves (`EDGES.md` row 66 — a `SET search_path` is accepted and never
/// reflected), this is a constant rather than a per-session read, and 18.6's answer
/// after `SET search_path TO ''` (NULL) or `TO pg_catalog, public` (`pg_catalog`)
/// is not reproducible here.
pub const CURRENT_SCHEMA: &str = "public";

/// `current_schemas(include_implicit)` — the search path as a list of the schemas
/// that actually exist.
///
/// `include_implicit` prepends `pg_catalog`, which Postgres searches whether or not
/// the list names it (the same fact [`IGNORABLE_SEARCH_PATH_ELEMENTS`] reads from
/// the other side). Measured on 18.6 under the default path: `true` →
/// `{pg_catalog,public}`, `false` → `{public}`.
#[must_use]
pub fn current_schemas(include_implicit: bool) -> Vec<&'static str> {
    if include_implicit {
        vec!["pg_catalog", CURRENT_SCHEMA]
    } else {
        vec![CURRENT_SCHEMA]
    }
}

/// `current_setting(name)`'s lookup — the **same** table `SHOW <name>` answers
/// from, so the function and the statement cannot disagree about a value or about
/// which names exist.
///
/// `None` is the unrecognized-parameter case, which the caller renders as 18.6's
/// `42704` or swallows for `missing_ok`. The name is matched case-insensitively,
/// as every GUC name is on 18.6 (measured: `current_setting('SEARCH_PATH')` and
/// `current_setting('Search_Path')` both answer).
#[must_use]
pub fn setting_value(name: &str, settings: &SessionSettings) -> Option<String> {
    ShowVariable::parse(&name.to_ascii_lowercase()).map(|var| var.value(settings).to_string())
}

/// The runtime parameters whose value belongs to the **session** rather than to the
/// build.
///
/// Everything else [`ShowVariable`] models is a fact about this engine —
/// UTC-only, UTF8-only, integer-datetimes-on — that no statement can move, so a
/// fixed string is the right answer for it. `application_name` is not: Postgres
/// carries it in the startup packet, reports it as a `GUC_REPORT` parameter, and
/// lets `SET` move it, so a fixed value here would be a claim about the client that
/// pglet has no business making. This is the one home for those, threaded to
/// [`execute_session_command`] the way `SequenceSession` is.
///
/// Only the native server can act on a *change* to one of these — the
/// `ParameterStatus` message is a wire frame, and the embed facade and the wasm
/// bindings have no wire. Both still carry the value, so `SHOW` answers the same
/// thing on every host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSettings {
    /// The current `application_name`, and the value a `RESET` restores.
    ///
    /// Postgres's reset target is the value the **startup packet** carried (an empty
    /// string when the client sent none), not a compile-time default — measured:
    /// after `SET application_name TO 'probe'`, 18.6's `DISCARD ALL` reports
    /// `application_name=` back to the connection that started with none.
    application_name: String,
    startup_application_name: String,
    /// The role this session runs as — what `current_user` / `session_user` /
    /// `user` / `current_role` answer.
    ///
    /// pglet models **no roles and no authentication**, so this is the name the
    /// startup packet claimed, echoed back. That is precisely what a real Postgres
    /// under `trust` authentication answers, which is the posture the differential
    /// harness compares against; it is a claim about the connection, not about a
    /// catalog. `SET ROLE` / `SET SESSION AUTHORIZATION` do not move it (they are
    /// unmodelled and stay in the GUC-echo family), so the two spellings that differ
    /// on 18.6 after a `SET ROLE` — `current_user` and `session_user` — cannot
    /// differ here.
    user: String,
    /// The database this session is connected to — what `current_database()` /
    /// `current_catalog` answer.
    ///
    /// A pglet database is a file or a memory image, not a named object in a
    /// cluster, so like [`Self::user`] this is the startup packet's claim echoed
    /// back. The embed and wasm hosts carry no packet and answer
    /// [`DEFAULT_SESSION_NAME`].
    database: String,
    /// The settings pglet acts on and a `SET` may move — the session's values
    /// ([`Gucs`]), what `RESET` restores, the open block's `SET LOCAL` overrides, and the
    /// open block's own state (its isolation level, and the values a rollback puts back).
    gucs: Gucs,
    reset: Gucs,
    local: LocalGucs,
    block: Option<BlockGucs>,
}

/// The session-level values of the settings pglet acts on.
///
/// - `lock_timeout` — how long, in milliseconds, a statement on the native server waits
///   for **the writer** (the one connection whose block may stage writes) before it is
///   refused `55P03`; `0` waits without a bound, as 18.6's `0` does. The embedded
///   `Database` never waits — it refuses at once — so there the value is only what `SHOW`
///   answers. **The default is pglet's, not
///   18.6's.** 18.6 defaults to `0` (wait forever); pglet defaults to
///   [`DEFAULT_LOCK_TIMEOUT_MS`], because a single writer makes an unrelated write wait
///   where 18.6 would not, and a finite bound turns a leaked block into a loud `55P03` at
///   the statement that waited rather than a stall. A host moves the default
///   ([`SessionSettings::set_default_lock_timeout`]): the native server's flag, an
///   embedding host's own setting. The startup packet's `lock_timeout` moves it for one
///   connection and becomes its `RESET` target.
/// - `default_transaction_isolation` — the level a block runs at when its `BEGIN` names
///   none.
///
/// **They are transactional, as 18.6's GUCs are** (`AtEOXact_GUC` / `AtEOSubXact_GUC`):
/// a `SET` inside a block is taken back by the block's rollback and by a `ROLLBACK TO` a
/// savepoint older than it, and kept by its commit and a `RELEASE`. `application_name`
/// is not here: 18.6 treats it the same way, and pglet does not yet (its change is
/// reported as a `ParameterStatus`, which a rollback would have to report again).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Gucs {
    lock_timeout: u64,
    default_isolation: IsolationLevel,
}

/// The open block's `SET LOCAL` overrides of [`Gucs`], which last to its end.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LocalGucs {
    lock_timeout: Option<u64>,
    default_isolation: Option<IsolationLevel>,
}

/// What the settings know of the open transaction block.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BlockGucs {
    /// The block's isolation level — `transaction_isolation`.
    isolation: IsolationLevel,
    /// Whether the block has run a statement that reads — 18.6's `FirstSnapshotSet`,
    /// after which its isolation level can no longer change.
    snapshot_taken: bool,
    /// The values each level of the block began with, outermost first: `levels[0]` is
    /// the block's, one more per open savepoint — what a rollback of that level puts
    /// back.
    levels: Vec<(Gucs, LocalGucs)>,
    /// When the block's transaction started — 18.6's `xactStartTimestamp`, which is the
    /// statement start its `BEGIN` ran under (`crate::builtins::statement_start`, in its
    /// form). `NO_INSTANT` where the clock could not be read, and the block's statements
    /// then read their own.
    started: i64,
}

/// pglet's default `lock_timeout`, in milliseconds — see `Gucs`.
pub const DEFAULT_LOCK_TIMEOUT_MS: u64 = 1000;

/// `lock_timeout`'s largest value, `INT_MAX` milliseconds — 18.6's `max`.
const LOCK_TIMEOUT_MAX_MS: i64 = i32::MAX as i64;

thread_local! {
    /// The current thread's session — see [`SessionSettings::install_ambient`].
    static AMBIENT_SESSION: std::cell::RefCell<SessionSettings> =
        std::cell::RefCell::new(SessionSettings::default());
}

/// What the `user` / `database` session facts answer on a host that carries no
/// startup packet — the embed facade and the wasm bindings.
///
/// Not `postgres`: a fabricated cluster identity would read as a fact about a
/// catalog pglet does not have. The native server never uses it (every connection
/// carries a packet, and `user` is required in one).
pub const DEFAULT_SESSION_NAME: &str = "pglet";

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            application_name: String::new(),
            startup_application_name: String::new(),
            user: DEFAULT_SESSION_NAME.to_string(),
            database: DEFAULT_SESSION_NAME.to_string(),
            gucs: Gucs::DEFAULT,
            reset: Gucs::DEFAULT,
            local: LocalGucs::default(),
            block: None,
        }
    }
}

impl Gucs {
    const DEFAULT: Self = Self {
        lock_timeout: DEFAULT_LOCK_TIMEOUT_MS,
        default_isolation: IsolationLevel::ReadCommitted,
    };
}

impl SessionSettings {
    /// The settings a connection starts with, given the `application_name` its
    /// startup packet carried (`""` when it carried none).
    ///
    /// The session identity stays [`DEFAULT_SESSION_NAME`]; a host with a startup
    /// packet calls [`Self::with_identity`] on the result.
    #[must_use]
    pub fn new(application_name: String) -> Self {
        Self {
            application_name: application_name.clone(),
            startup_application_name: application_name,
            ..Self::default()
        }
    }

    /// Record the startup packet's `user` and `database` (the native server's).
    #[must_use]
    pub fn with_identity(mut self, user: String, database: String) -> Self {
        self.user = user;
        self.database = database;
        self
    }

    /// The live `application_name`.
    #[must_use]
    pub fn application_name(&self) -> &str {
        &self.application_name
    }

    /// The session's role — see [`Self::user`].
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// The session's database — see [`Self::database`].
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// The `lock_timeout` in force, in milliseconds (`0`: wait without a bound) — a
    /// `SET LOCAL`'s while its block lasts, else the session's.
    #[must_use]
    pub fn lock_timeout_ms(&self) -> u64 {
        self.local.lock_timeout.unwrap_or(self.gucs.lock_timeout)
    }

    /// Make `ms` this session's `lock_timeout` and its `RESET` target — a host's own
    /// default (the native server's flag, an embedding host's setting). It is not a
    /// `SET`: an open block's rollback does not take it back, so it replaces the value
    /// each level of the block would put back as well.
    pub fn set_default_lock_timeout(&mut self, ms: u64) {
        self.gucs.lock_timeout = ms;
        self.reset.lock_timeout = ms;
        self.local.lock_timeout = None;
        if let Some(block) = &mut self.block {
            for (gucs, local) in &mut block.levels {
                gucs.lock_timeout = ms;
                local.lock_timeout = None;
            }
        }
    }

    /// Apply the startup packet's `lock_timeout`, which becomes the session's value and
    /// its `RESET` target, as a packet's value does on 18.6.
    ///
    /// # Errors
    ///
    /// `22023` for a value `lock_timeout` refuses (see [`parse_lock_timeout`]).
    pub fn apply_startup_lock_timeout(&mut self, value: &str) -> Result<()> {
        let ms = parse_lock_timeout(value)?;
        self.set_default_lock_timeout(ms);
        Ok(())
    }

    /// Apply the startup packet's `default_transaction_isolation`, which becomes the
    /// session's value and its `RESET` target.
    ///
    /// # Errors
    ///
    /// `22023` for a value that names no level (see [`parse_isolation_setting`]).
    pub fn apply_startup_default_isolation(&mut self, value: &str) -> Result<()> {
        let level = parse_isolation_setting("default_transaction_isolation", value)?;
        self.gucs.default_isolation = level;
        self.reset.default_isolation = level;
        self.local.default_isolation = None;
        Ok(())
    }

    /// `default_transaction_isolation` in force — a `SET LOCAL`'s while its block lasts,
    /// else the session's.
    #[must_use]
    pub fn default_isolation(&self) -> IsolationLevel {
        self.local
            .default_isolation
            .unwrap_or(self.gucs.default_isolation)
    }

    /// `transaction_isolation`: the open block's level, and outside one the level the
    /// next block would begin at — what 18.6's `SHOW` answers from the implicit
    /// transaction it runs in.
    #[must_use]
    pub fn transaction_isolation(&self) -> IsolationLevel {
        self.block
            .as_ref()
            .map_or_else(|| self.default_isolation(), |block| block.isolation)
    }

    /// A block opened — `BEGIN`, at `requested` or at `default_transaction_isolation`.
    /// With a block already open, the `BEGIN` is 18.6's warning-and-continue, and a level
    /// it names is set on the open block as `SET TRANSACTION` would set it.
    ///
    /// # Errors
    ///
    /// `25001` where a named level would change the open block's after it read
    /// (`Self::set_block_isolation`).
    pub fn begin_block(&mut self, requested: Option<IsolationLevel>) -> Result<()> {
        if self.block.is_some() {
            return requested.map_or(Ok(()), |level| self.set_block_isolation(level));
        }
        self.block = Some(BlockGucs {
            isolation: requested.unwrap_or_else(|| self.default_isolation()),
            snapshot_taken: false,
            levels: vec![(self.gucs, self.local)],
            started: crate::builtins::statement_start(),
        });
        Ok(())
    }

    /// The open block ran a statement that reads — after it, its isolation level is
    /// fixed. The hosts call it for every planned statement a block runs but
    /// transaction control, which takes no snapshot on 18.6 either.
    pub const fn note_block_statement(&mut self) {
        if let Some(block) = &mut self.block {
            block.snapshot_taken = true;
        }
    }

    /// A savepoint opened in the block: what a `ROLLBACK TO` it puts back.
    pub fn savepoint_block(&mut self) {
        let saved = (self.gucs, self.local);
        if let Some(block) = &mut self.block {
            block.levels.push(saved);
        }
    }

    /// A `ROLLBACK TO` left the block `depth` levels deep (the savepoint it rolled back
    /// to being the innermost): the values that savepoint began with are put back.
    pub fn rollback_to_block(&mut self, depth: usize) {
        let Some(block) = &mut self.block else {
            return;
        };
        block.levels.truncate(depth);
        if let Some(&(gucs, local)) = block.levels.last() {
            self.gucs = gucs;
            self.local = local;
        }
    }

    /// A `RELEASE` left the block `depth` levels deep: the released levels' changes are
    /// kept, merged into the level that holds them now.
    pub fn release_block(&mut self, depth: usize) {
        if let Some(block) = &mut self.block {
            block.levels.truncate(depth.max(1));
        }
    }

    /// The block ended: a `SET LOCAL` lasts no longer, and a rollback — of any kind, an
    /// aborted block's `COMMIT` and a refused one included — puts back what the block
    /// began with. A no-op with no block open.
    pub fn end_block(&mut self, committed: bool) {
        if let Some(block) = self.block.take() {
            if !committed && let Some(&(gucs, _)) = block.levels.first() {
                self.gucs = gucs;
            }
            self.local = LocalGucs::default();
        }
    }

    /// Whether the open block runs at a level that keeps one snapshot for its length —
    /// `REPEATABLE READ` or `SERIALIZABLE`.
    #[must_use]
    pub fn block_uses_transaction_snapshot(&self) -> bool {
        self.block
            .as_ref()
            .is_some_and(|block| block.isolation.uses_transaction_snapshot())
    }

    /// `SET TRANSACTION ISOLATION LEVEL` (and `SET transaction_isolation`) in the open
    /// block, with 18.6's `check_transaction_isolation`: a level that would change after
    /// the block read, or inside a savepoint, is refused. Outside a block it changes
    /// nothing, as 18.6's does (which also warns).
    ///
    /// # Errors
    ///
    /// `25001` for the two refusals.
    fn set_block_isolation(&mut self, level: IsolationLevel) -> Result<()> {
        let Some(block) = &mut self.block else {
            return Ok(());
        };
        if level == block.isolation {
            return Ok(());
        }
        if block.snapshot_taken {
            return Err(Error::TransactionIsolationFixed(
                "SET TRANSACTION ISOLATION LEVEL must be called before any query",
            ));
        }
        if block.levels.len() > 1 {
            return Err(Error::TransactionIsolationFixed(
                "SET TRANSACTION ISOLATION LEVEL must not be called in a subtransaction",
            ));
        }
        block.isolation = level;
        Ok(())
    }

    /// Restore every session setting to what the startup packet established —
    /// `RESET ALL` and the `RESET ALL` inside `DISCARD ALL`.
    fn reset_all(&mut self) {
        self.application_name
            .clone_from(&self.startup_application_name);
        self.gucs = self.reset;
        self.local = LocalGucs::default();
    }

    /// Install this session as the **ambient** one for the current thread, so the
    /// expression evaluator can read it.
    ///
    /// `current_user`, `current_database()` and `current_setting(…)` are ordinary
    /// expressions: they are evaluated deep inside the executor — inside a stored
    /// column `DEFAULT`, a `CHECK`, an `ADD COLUMN` backfill — at sites that are
    /// handed a row and a scope and nothing else, and threading a session object to
    /// every one of them would be a wide change that a single missed site turns into
    /// a wrong answer rather than a compile error. The clock and the RNG behind
    /// `now()` / `gen_random_uuid()` are already ambient for the same reason
    /// (`crate::builtins`); this puts the session beside them. It carries the open
    /// block's start with it, which is what the transaction clock answers inside a
    /// block (`crate::builtins::set_block_start`).
    ///
    /// **The thread *is* the connection** on every host pglet has: the native server
    /// is thread-per-connection, wasm is single-threaded, and the embed facade's
    /// `Database` is `&mut self` per statement. It is re-installed **per statement**
    /// rather than once per connection, so a thread that serves a second connection
    /// cannot answer with the first one's identity, and a `SET application_name`
    /// cannot leave a stale copy behind.
    ///
    /// # The install sites
    ///
    /// **This list is the one home for it**, and every other site points here rather
    /// than restating a count. It has already gone stale once: it said *two* while a
    /// third route — COPY — was writing rows against another session's identity, and
    /// four comments carried the wrong number together, which is exactly the class
    /// `CLAUDE.md` states as *the reader enumeration is what has gone stale; re-run
    /// it, don't inherit it*. Re-derive it as "every route that reaches a statement
    /// **without** passing through one of the sites above it", not by decrementing.
    ///
    /// 1. [`execute_session_command`] — the native server's simple-query loop, and the
    ///    embed/wasm driver's path for a statement it does not parse first.
    /// 2. `pglet_server::execute::execute_query_core` — both native protocols, and
    ///    the only site an extended-protocol **cached portal** reaches (it returns
    ///    before the statement loop that would hit site 1).
    /// 3. `pglet_server::copy::dispatch_copy` — the native COPY interceptors return
    ///    before `execute_query_core`, so neither site above sees a COPY. It
    ///    **writes**, which makes this the one whose absence was a wrong row.
    /// 4. `pglet::Database::copy_in` / `copy_out` — the embed facade's COPY entry
    ///    points, which bypass `execute_as`/`execute_bound` and so site 1.
    ///
    /// 5. `pglet::Database::execute_as` / `execute_bound`'s parse-first route — a text
    ///    with no `;` outside an aborted block is parsed ahead of site 1, and reaches it
    ///    only when that parse fails.
    ///
    /// Sites 3 and 4 are the same shape at two hosts; 4 is currently unobservable
    /// (the facade exposes no way to set an identity, so every embed session answers
    /// the one [`DEFAULT_SESSION_NAME`] constant) and is installed anyway, because it
    /// becomes site 3's defect the day the facade takes one.
    pub fn install_ambient(&self) {
        AMBIENT_SESSION.with(|cell| cell.replace(self.clone()));
        crate::builtins::set_block_start(
            self.block
                .as_ref()
                .map_or(crate::builtins::NO_INSTANT, |block| block.started),
        );
    }

    /// Read the current thread's ambient session — see [`Self::install_ambient`].
    ///
    /// A thread that has installed none reads [`SessionSettings::default`], which is
    /// the right answer for a caller with no session (a test constructing an
    /// executor directly, a stored-expression probe).
    #[must_use]
    pub fn ambient() -> Self {
        AMBIENT_SESSION.with(|cell| cell.borrow().clone())
    }

    /// Apply a `SET`/`RESET` of one modelled session setting; `None` resets it.
    ///
    /// Returns `false` when `name` is not one, which is what keeps the GUC-echo
    /// family a no-op: an unmodelled name is accepted and forgotten, exactly as
    /// before. `scope` is the statement's: a `SET LOCAL` inside a block lasts to the
    /// block's end ([`Self::end_block`]), and outside one changes nothing, as on
    /// 18.6 (which warns); `application_name` keeps its session reading of both.
    ///
    /// # Errors
    ///
    /// `22023` for a value the setting's type refuses; `0A000` for a reset of
    /// `transaction_isolation`, which 18.6 refuses; `25001` for a
    /// `transaction_isolation` the open block can no longer take.
    fn assign(&mut self, name: &str, value: Option<String>, scope: SetScope) -> Result<bool> {
        match name {
            "application_name" => {
                self.application_name =
                    value.unwrap_or_else(|| self.startup_application_name.clone());
                Ok(true)
            }
            "lock_timeout" => {
                let ms = match value {
                    Some(v) => parse_lock_timeout(&v)?,
                    None => self.reset.lock_timeout,
                };
                match scope {
                    SetScope::LocalInBlock => self.local.lock_timeout = Some(ms),
                    SetScope::LocalOutsideBlock => {}
                    SetScope::Session => {
                        self.gucs.lock_timeout = ms;
                        self.local.lock_timeout = None;
                    }
                }
                Ok(true)
            }
            "default_transaction_isolation" => {
                let level = match value {
                    Some(v) => parse_isolation_setting(name, &v)?,
                    None => self.reset.default_isolation,
                };
                self.assign_default_isolation(level, scope);
                Ok(true)
            }
            "transaction_isolation" => {
                let Some(value) = value else {
                    return Err(Error::NotImplemented(
                        "parameter \"transaction_isolation\" cannot be reset".into(),
                    ));
                };
                let level = parse_isolation_setting(name, &value)?;
                self.set_block_isolation(level)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `default_transaction_isolation := level` at `scope`.
    const fn assign_default_isolation(&mut self, level: IsolationLevel, scope: SetScope) {
        match scope {
            SetScope::LocalInBlock => self.local.default_isolation = Some(level),
            SetScope::LocalOutsideBlock => {}
            SetScope::Session => {
                self.gucs.default_isolation = level;
                self.local.default_isolation = None;
            }
        }
    }
}

/// Read an isolation-level setting's value, as 18.6 reads an enum GUC.
///
/// # Errors
///
/// `22023` — `invalid value for parameter "<name>": "<value>"`, with the hint listing
/// the four levels — for a value that names none.
pub fn parse_isolation_setting(name: &str, value: &str) -> Result<IsolationLevel> {
    IsolationLevel::from_setting(value).ok_or_else(|| Error::InvalidParameterValue {
        message: format!("invalid value for parameter \"{name}\": \"{value}\""),
        hint: Some(format!(
            "Available values: {}.",
            IsolationLevel::ALL.map(IsolationLevel::name).join(", ")
        )),
    })
}

/// How long an assignment lasts — `SET` / `RESET` against `SET LOCAL`, and whether a
/// block is open for the latter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetScope {
    Session,
    LocalInBlock,
    LocalOutsideBlock,
}

/// Parse a `lock_timeout` value into milliseconds, as 18.6's `parse_int` reads an
/// integer GUC.
///
/// A number — `strtol`'s, so `0x…` is hex and a leading `0` octal — or, where it stops
/// at `.`, `e` or `E`, a decimal one, then optional whitespace and a unit (`us`, `ms`,
/// `s`, `min`, `h`, `d`), a fraction rounded to the next smaller unit, the whole rounded
/// to a millisecond and checked against `0 .. INT_MAX`.
///
/// # Errors
///
/// `22023` — `invalid value for parameter "lock_timeout": "…"` for a value that does
/// not parse, with 18.6's units hint where a unit was the trouble, and `… ms is outside
/// the valid range …` for one past the bounds.
pub fn parse_lock_timeout(value: &str) -> Result<u64> {
    const UNITS_HINT: &str =
        "Valid units for this parameter are \"us\", \"ms\", \"s\", \"min\", \"h\", and \"d\".";
    let text = value;
    let invalid = |hint: Option<&str>| Error::InvalidParameterValue {
        message: format!("invalid value for parameter \"lock_timeout\": \"{text}\""),
        hint: hint.map(str::to_string),
    };
    let (number, rest) = parse_guc_number(text).ok_or_else(|| invalid(None))?;
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let ms = if rest.is_empty() {
        number
    } else {
        let unit = rest.trim_end_matches(|c: char| c.is_ascii_whitespace());
        // (unit, milliseconds, the next smaller unit's milliseconds)
        let table: [(&str, f64, Option<f64>); 6] = [
            ("d", 86_400_000.0, Some(3_600_000.0)),
            ("h", 3_600_000.0, Some(60_000.0)),
            ("min", 60_000.0, Some(1000.0)),
            ("s", 1000.0, Some(1.0)),
            ("ms", 1.0, Some(0.001)),
            ("us", 0.001, None),
        ];
        let Some(&(_, multiplier, next)) = table.iter().find(|(name, ..)| *name == unit) else {
            return Err(invalid(Some(UNITS_HINT)));
        };
        let converted = number * multiplier;
        next.map_or(converted, |next| {
            (converted / next).round_ties_even() * next
        })
    };
    let ms = ms.round_ties_even();
    if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&ms) {
        return Err(invalid(Some("Value exceeds integer range.")));
    }
    // In range for an `i32` by the check above.
    #[allow(clippy::cast_possible_truncation)]
    let ms = ms as i64;
    if !(0..=LOCK_TIMEOUT_MAX_MS).contains(&ms) {
        return Err(Error::InvalidParameterValue {
            message: format!(
                "{ms} ms is outside the valid range for parameter \"lock_timeout\" (0 ms .. {LOCK_TIMEOUT_MAX_MS} ms)"
            ),
            hint: None,
        });
    }
    Ok(ms.unsigned_abs())
}

/// The leading number of a GUC value, as `parse_int` reads it — `strtol(…, 0)`, then
/// `strtod` where that stops at `.`, `e` or `E` — and the text after it. `None` where no
/// number leads.
fn parse_guc_number(text: &str) -> Option<(f64, &str)> {
    let start = text.len()
        - text
            .trim_start_matches(|c: char| c.is_ascii_whitespace())
            .len();
    let body = &text[start..];
    let (negative, unsigned) = match body.as_bytes().first() {
        Some(b'-') => (true, &body[1..]),
        Some(b'+') => (false, &body[1..]),
        _ => (false, body),
    };
    let sign_len = body.len() - unsigned.len();
    // `strtol` with base 0: `0x`/`0X` then hex digits, a leading `0` octal, else decimal.
    let (radix, digits_at) = if unsigned.len() > 2
        && (unsigned.starts_with("0x") || unsigned.starts_with("0X"))
        && unsigned.as_bytes()[2].is_ascii_hexdigit()
    {
        (16, 2)
    } else if unsigned.starts_with('0') {
        (8, 0)
    } else {
        (10, 0)
    };
    let digits = &unsigned[digits_at..];
    let len = digits
        .find(|c: char| !c.is_digit(radix))
        .unwrap_or(digits.len());
    if len == 0 {
        // No integer at all: `strtod` may still read one (`.5`).
        return parse_guc_decimal(text, start);
    }
    let end = start + sign_len + digits_at + len;
    let after = &text[end..];
    if after.starts_with(['.', 'e', 'E']) || i64::from_str_radix(&digits[..len], radix).is_err() {
        return parse_guc_decimal(text, start);
    }
    let magnitude = i64::from_str_radix(&digits[..len], radix).ok()?;
    // `strtol` overflows past `long`, which `parse_int` then re-reads with `strtod`.
    #[allow(clippy::cast_precision_loss)]
    let value = if negative { -magnitude } else { magnitude } as f64;
    Some((value, after))
}

/// `strtod`'s reading of the decimal number at `text[start..]`: sign, digits, an
/// optional fraction, an optional exponent.
fn parse_guc_decimal(text: &str, start: usize) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    let mut i = start;
    if matches!(bytes.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let int_start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - int_start;
    if bytes.get(i) == Some(&b'.') {
        let frac_start = i + 1;
        let mut j = frac_start;
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        digits += j - frac_start;
        if digits > 0 {
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'-' | b'+')) {
            j += 1;
        }
        let exp_start = j;
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    let value: f64 = text[start..i].parse().ok()?;
    if value.is_nan() {
        return None;
    }
    Some((value, &text[i..]))
}

/// `lock_timeout` as `SHOW` renders it — 18.6's `convert_int_from_base_unit`.
///
/// `0` bare, otherwise the largest of `d`, `h`, `min`, `s` that divides it evenly, else
/// `ms`, with no space (`1s`, `1500ms`, `1min`).
#[must_use]
pub fn format_lock_timeout(ms: u64) -> String {
    if ms == 0 {
        return "0".to_string();
    }
    for (unit, size) in [
        ("d", 86_400_000),
        ("h", 3_600_000),
        ("min", 60_000),
        ("s", 1000),
    ] {
        if ms.is_multiple_of(size) {
            return format!("{}{unit}", ms / size);
        }
    }
    format!("{ms}ms")
}

/// Handle SET commands.
///
/// Every `SET <guc> …` pglet does not model is an accepted no-op reporting tag
/// `SET` — the GUC-echo family, which is what lets a driver's connection-setup
/// batch through. **Three shapes are not that**, and each is here because echoing
/// it would be a lie about what the session then does:
///
/// - `SET [SESSION | LOCAL] search_path …` naming a schema pglet does not have is
///   refused — a schema-per-test harness would believe it is isolated and read and
///   write the one shared schema. See [`check_search_path_elements`], which is the
///   one decider **every** route to this GUC reaches, statement or function.
/// - a transaction-mode `SET` asking for `READ ONLY` is refused — `SET TRANSACTION
///   READ ONLY` and `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`. pglet
///   has no read-only block to enter, and accepting the request would let the write
///   it forbids succeed. `parser::ddl`'s `parse_transaction_modes` refuses the same
///   words on the `BEGIN` / `START TRANSACTION` side.
/// - `SET application_name …` is **recorded**, not forgotten: it is a session value
///   Postgres reports at connect and after every change, so a fixed answer here
///   would be a claim about the client. See [`SessionSettings`].
///
/// # Errors
///
/// `0A000` for either refusal.
pub fn execute_set_command(
    sql: &str,
    settings: &mut SessionSettings,
    in_transaction_block: bool,
) -> Result<QueryResult> {
    check_set_command(sql, settings, in_transaction_block)?;
    Ok(QueryResult {
        shape: ResultShape::NoResultSet,
        rows: vec![],
        command_tag: "SET".into(),
        notices: Vec::new(),
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

/// The refusals and the one recording [`execute_set_command`] makes, over the
/// leading tokens.
///
/// Read from the **lexer** rather than the raw text, which is where a quoted
/// spelling (`SET "search_path" TO nosuch`) and a string-valued one
/// (`SET search_path = 'nosuch'`) are the same statement as the bare form. The
/// classifier that got here ([`is_set_command`]) is still a raw-text prefix test —
/// that is its own documented fold-width contract, and this reader does not change
/// it.
fn check_set_command(
    sql: &str,
    settings: &mut SessionSettings,
    in_transaction_block: bool,
) -> Result<()> {
    // The **whole** statement, and a lex error is this statement's answer rather than a
    // reason to read less. Both halves were once wrong at the same seam: a capped read
    // accepted `SET search_path TO <31 × public>, evil_schema` on the prefix it could
    // see, and then — with the cap refusing — refused a 40-element all-`public` list
    // that 18.6 accepts and that the `set_config` route (which reads a whole string)
    // took, so the two routes disagreed on a valid request. A silent stop at a lex error
    // was the third: `SET search_path TO ""` left this reader looking at
    // `SET search_path TO` and answering `0A000` where the scanner's own `42601` is the
    // statement's error.
    let mut tokens = all_tokens(sql).map_err(|e| lex_error_to_error(e, sql))?;
    let end = tokens
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Semicolon))
        .unwrap_or(tokens.len());
    let mut rest = tokens.get(1..end).unwrap_or_default();
    // `SET LOCAL …` / `SET SESSION …` — the scope noise words. `SET SESSION
    // CHARACTERISTICS …` starts the same way and is handled by the branch below,
    // which is why this consumes the word without deciding anything.
    let scope_word = rest
        .first()
        .filter(|t| is_word(t, sql, "local") || is_word(t, sql, "session"));
    let scope = match scope_word {
        Some(t) if is_word(t, sql, "local") && in_transaction_block => SetScope::LocalInBlock,
        Some(t) if is_word(t, sql, "local") => SetScope::LocalOutsideBlock,
        _ => SetScope::Session,
    };
    if scope_word.is_some() {
        rest = &rest[1..];
    }
    let Some(head) = rest.first() else {
        return Ok(());
    };
    // `SET [SESSION|LOCAL] TRANSACTION <modes>` and `SET SESSION CHARACTERISTICS AS
    // TRANSACTION <modes>` are transaction-mode statements wearing the `SET`
    // prefix, not GUC assignments.
    //
    // Both carry an isolation level pglet acts on: `SET TRANSACTION` sets the open
    // block's (and outside a block changes nothing, as on 18.6), `SET SESSION
    // CHARACTERISTICS` sets `default_transaction_isolation`.
    if matches!(head.kind, TokenKind::Keyword(Keyword::Transaction)) {
        check_transaction_mode_words(&rest[1..], sql)?;
        if let Some(level) = transaction_mode_isolation(&rest[1..], sql)? {
            settings.set_block_isolation(level)?;
        }
        return Ok(());
    }
    if is_word(head, sql, "characteristics") {
        check_transaction_mode_words(&rest[1..], sql)?;
        if let Some(level) = transaction_mode_isolation(&rest[1..], sql)? {
            settings.assign_default_isolation(level, scope);
        }
        return Ok(());
    }
    // A GUC name is matched case-insensitively whatever its quoting, as 18.6's `find_option`
    // matches it (`guc_name_compare`): `SET "Quote_All_Identifiers" = on` names the same setting
    // as the bare spelling, so it must reach the same decider. `Token::name` keeps a quoted
    // spelling verbatim, which let a mixed-case quoted name walk past every check below.
    let names_spelling = head.name(sql) == "names";
    let head_name = head.name(sql).to_ascii_lowercase();
    // `SET [SESSION|LOCAL] TIME ZONE <zone_value>` is `gram.y`'s other spelling of `SET
    // timezone = <zone_value>`, its `LOCAL` the other's `DEFAULT`. The tokens are rewritten
    // into that statement, so the reader and the decider below serve both spellings.
    let time_zone =
        is_word(head, sql, "time") && rest.get(1).is_some_and(|t| is_word(t, sql, "zone"));
    if time_zone {
        let local = rest.get(2).is_some_and(|t| is_word(t, sql, "local"));
        let at = end - rest.len();
        if let Some([_, zone, value @ ..]) = tokens.get_mut(at..end) {
            zone.kind = TokenKind::Eq;
            if local && let Some(word) = value.first_mut() {
                word.kind = TokenKind::Keyword(Keyword::Default);
            }
        }
        rest = tokens.get(at..end).unwrap_or_default();
    }
    let name = if time_zone {
        "timezone".to_string()
    } else {
        head_name
    };
    if name == "search_path" {
        return check_search_path_statement(&rest[1..], sql);
    }
    // `SET NAMES <encoding>` is `gram.y`'s other spelling of `SET client_encoding`
    // (`set_rest_more: … | NAMES opt_encoding`), so it reaches the same decider — a
    // gate with a spelling that walks past it is the defect this fold closes.
    let name = if name == "names" {
        "client_encoding".to_string()
    } else {
        name
    };
    let assigned = if name == "datestyle"
        && let [assign, first, _, .., last] = &rest[1..]
        && matches!(assign.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq)
    {
        // `DateStyle` is a list (`SET DateStyle = ISO, MDY`), which reaches the decider as
        // its source text: no word it honors is spelled with a quote.
        Some(SetValue::To(
            sql.get(first.start..last.end)
                .unwrap_or_default()
                .to_string(),
        ))
    } else if names_spelling {
        // `gram.y`'s `NAMES opt_encoding`: a string constant, `DEFAULT` (as `SET
        // client_encoding TO DEFAULT`), or nothing — one token, with no `TO`/`=`. With a
        // `TO`/`=` the statement is `generic_set` instead, naming a setting called `names`,
        // which 18.6 does not have (`42704`); any other token is 18.6's syntax error at it,
        // spelled as written.
        let at = |t: &Token| {
            Error::syntax_sentence_at(
                format!("syntax error at or near \"{}\"", &sql[t.start..t.end]),
                pglet_core::codepoint_position(sql, t.start),
            )
        };
        match rest.get(1) {
            Some(t) if matches!(t.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq) => {
                return Err(names_generic_set(&rest[2..], tokens.get(end), sql));
            }
            Some(t) if unquoted_word(t, sql, "from") => {
                // `set_rest_more`'s `var_name FROM CURRENT`, naming the setting `names`.
                return Err(match &rest[2..] {
                    [] => unexpected_end(tokens.get(end), sql),
                    [current] if unquoted_word(current, sql, "current") => {
                        Error::UnrecognizedParameter("names".to_string())
                    }
                    [current, extra, ..] if unquoted_word(current, sql, "current") => at(extra),
                    [other, ..] => at(other),
                });
            }
            Some(t)
                if !matches!(
                    t.kind,
                    TokenKind::String | TokenKind::Keyword(Keyword::Default)
                ) =>
            {
                return Err(at(t));
            }
            _ => {}
        }
        if let Some(extra) = rest.get(2) {
            return Err(at(extra));
        }
        rest.get(1).map(|t| {
            if matches!(t.kind, TokenKind::Keyword(Keyword::Default)) {
                SetValue::Default
            } else {
                SetValue::To(t.text(sql).to_string())
            }
        })
    } else if name == "lock_timeout" || guc_value_is_checked(&name) {
        // A value pglet acts on or decides: one it cannot read is refused, never echoed
        // and forgotten — a list or a signed number included, which the decider must see.
        Some(numeric_only_statement_value(&name, &rest[1..], sql)?)
    } else if name == "default_transaction_isolation" || name == "transaction_isolation" {
        Some(acted_on_statement_value(&name, &rest[1..], sql)?)
    } else {
        set_statement_value(&rest[1..], sql)
    };
    check_guc_value(
        &name,
        match &assigned {
            Some(SetValue::To(v)) => Some(v.as_str()),
            _ => None,
        },
    )?;
    if let Some(value) = assigned {
        settings.assign(
            &name,
            match value {
                SetValue::Default => None,
                SetValue::To(v) => Some(v),
            },
            scope,
        )?;
    }
    Ok(())
}

/// Whether `t` is `word` unquoted, lexed as an identifier or as one of pglet's keywords.
fn unquoted_word(t: &Token, sql: &str, word: &str) -> bool {
    match t.kind {
        TokenKind::Keyword(k) => k.as_str() == word,
        _ => is_word(t, sql, word),
    }
}

/// What 18.6 answers for `SET NAMES {TO|=} <list>` — `gram.y`'s `generic_set` naming a setting
/// called `names`, which it does not have, **after** the list is parsed: `var_list`'s syntax
/// first (`42601` at the first token it refuses, or at the end), then `22023 SET names takes only
/// one argument` for a list of more than one value, and only then `42704`. A value is a string,
/// a signed or unsigned number, `TRUE` / `FALSE` / `ON`, or any word but a reserved keyword
/// (`opt_boolean_or_string | NumericOnly`); `DEFAULT` stands alone. The reserved words are the
/// ones pglet's lexer knows: a word 18.6 reserves that it has no keyword for (`user`,
/// `current_user`, `window`, …) arrives as an identifier and passes as a value (`42704` here,
/// `42601` there), and a bit or hex literal (`B'01'`) is refused at its quote rather than its
/// letter (`EDGES.md` row 256).
fn names_generic_set(list: &[Token], terminator: Option<&Token>, sql: &str) -> Error {
    let at = |t: &Token| {
        Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", &sql[t.start..t.end]),
            pglet_core::codepoint_position(sql, t.start),
        )
    };
    let is_value_word = |t: &Token| match t.kind {
        TokenKind::String
        | TokenKind::Number
        | TokenKind::Identifier
        | TokenKind::QuotedIdentifier
        | TokenKind::Keyword(Keyword::True | Keyword::False | Keyword::On) => true,
        TokenKind::Keyword(k) => !matches!(k.pg_class(), crate::lexer::PgClass::Reserved),
        _ => false,
    };
    if let [only] = list
        && matches!(only.kind, TokenKind::Keyword(Keyword::Default))
    {
        return Error::UnrecognizedParameter("names".to_string());
    }
    if let [default, extra, ..] = list
        && matches!(default.kind, TokenKind::Keyword(Keyword::Default))
    {
        return at(extra);
    }
    let mut values = 0usize;
    let mut i = 0;
    loop {
        // one value: an optional sign before a number, or a value word
        match list.get(i) {
            None => return unexpected_end(terminator, sql),
            Some(sign) if matches!(sign.kind, TokenKind::Minus | TokenKind::Plus) => {
                match list.get(i + 1) {
                    Some(n) if matches!(n.kind, TokenKind::Number) => i += 2,
                    Some(other) => return at(other),
                    None => return unexpected_end(terminator, sql),
                }
            }
            Some(t) if is_value_word(t) => i += 1,
            Some(other) => return at(other),
        }
        values += 1;
        match list.get(i) {
            None => break,
            Some(comma) if matches!(comma.kind, TokenKind::Comma) => i += 1,
            Some(other) => return at(other),
        }
    }
    if values > 1 {
        return Error::InvalidParameterValue {
            message: "SET names takes only one argument".to_string(),
            hint: None,
        };
    }
    Error::UnrecognizedParameter("names".to_string())
}

/// The value a `SET <name> {TO|=} <value>` assigns to `lock_timeout` or to a setting
/// [`check_guc_value`] decides: [`set_statement_value`]'s one-token shapes, and a signed
/// number, which the lexer reads as two tokens — the grammar's `NumericOnly` — so that `-1`
/// reaches the check 18.6 answers it with (`lock_timeout`'s range, `quote_all_identifiers`'
/// Boolean reader).
///
/// # Errors
///
/// `22023` for a list (18.6's `SET <name> takes only one argument`), `42601` for any other
/// shape: pglet acts on or decides this setting, so a value it cannot read must not pass as
/// accepted.
fn numeric_only_statement_value(name: &str, value: &[Token], sql: &str) -> Result<SetValue> {
    if let Some(assigned) = set_statement_value(value, sql) {
        return Ok(assigned);
    }
    check_one_argument(name, value)?;
    if let [assign, sign, number] = value
        && matches!(assign.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq)
        && matches!(sign.kind, TokenKind::Minus | TokenKind::Plus)
        && matches!(number.kind, TokenKind::Number)
    {
        let negative = matches!(sign.kind, TokenKind::Minus);
        return Ok(SetValue::To(numeric_only_text(negative, number.text(sql))));
    }
    Err(Error::syntax(format!(
        "syntax error in SET {name}: expected one value"
    )))
}

/// The text a `SET` value's number carries, as 18.6's grammar builds it: an integer literal
/// that fits `int4` (the scanner's `ICONST`, underscores and leading zeros read away) is the
/// integer, negated for a `-` — so `01` and `0_1` are `1` and `-0` is `0`, and
/// `quote_all_identifiers = 01` is on, measured — and any other number (`FCONST`, an integer
/// past `int4` included) is its spelling, a `-` prefixed.
fn numeric_only_text(negative: bool, text: &str) -> String {
    let digits: String = text.chars().filter(|&c| c != '_').collect();
    if !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        && let Ok(n) = digits.parse::<i32>()
    {
        return if negative {
            (-i64::from(n)).to_string()
        } else {
            n.to_string()
        };
    }
    if negative {
        format!("-{text}")
    } else {
        text.to_string()
    }
}

/// The value a `SET <name> {TO|=} <value>` assigns to a setting pglet acts on whose
/// value is one word or string — [`set_statement_value`]'s shapes and nothing else.
///
/// # Errors
///
/// `22023` for a list (18.6's `SET <name> takes only one argument`), `42601` for any
/// other shape: a value pglet cannot read must not pass as accepted.
fn acted_on_statement_value(name: &str, value: &[Token], sql: &str) -> Result<SetValue> {
    if let Some(assigned) = set_statement_value(value, sql) {
        return Ok(assigned);
    }
    check_one_argument(name, value)?;
    Err(Error::syntax(format!(
        "syntax error in SET {name}: expected one value"
    )))
}

/// Refuse a comma-separated list assigned to a setting that takes one value — 18.6's
/// `flatten_set_variable_args`, which answers before the value is read.
///
/// # Errors
///
/// `22023` — `SET <name> takes only one argument`.
fn check_one_argument(name: &str, value: &[Token]) -> Result<()> {
    let is_list = value.split_first().is_some_and(|(assign, items)| {
        matches!(assign.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq)
            && items.iter().any(|t| matches!(t.kind, TokenKind::Comma))
    });
    if is_list {
        return Err(Error::InvalidParameterValue {
            message: format!("SET {name} takes only one argument"),
            hint: None,
        });
    }
    Ok(())
}

/// The isolation level a `SET TRANSACTION` / `SET SESSION CHARACTERISTICS AS
/// TRANSACTION` mode list names, the last one winning — `gram.y`'s `iso_level` after
/// the words `ISOLATION LEVEL`.
///
/// # Errors
///
/// `42601` for `ISOLATION LEVEL` followed by no level.
fn transaction_mode_isolation(words: &[Token], sql: &str) -> Result<Option<IsolationLevel>> {
    let mut level = None;
    let mut i = 0;
    while i < words.len() {
        if is_word(&words[i], sql, "isolation")
            && words.get(i + 1).is_some_and(|t| is_word(t, sql, "level"))
        {
            let at = |k: usize, word: &str| words.get(i + k).is_some_and(|t| is_word(t, sql, word));
            let (named, len) = if at(2, "serializable") {
                (IsolationLevel::Serializable, 1)
            } else if at(2, "repeatable") && at(3, "read") {
                (IsolationLevel::RepeatableRead, 2)
            } else if at(2, "read") && at(3, "committed") {
                (IsolationLevel::ReadCommitted, 2)
            } else if at(2, "read") && at(3, "uncommitted") {
                (IsolationLevel::ReadUncommitted, 2)
            } else {
                return Err(Error::syntax(
                    "syntax error in SET TRANSACTION: expected an isolation level".to_string(),
                ));
            };
            level = Some(named);
            i += 2 + len;
            continue;
        }
        i += 1;
    }
    Ok(level)
}

/// What a `SET <name> {TO|=} <value>` assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SetValue {
    /// `SET <name> TO DEFAULT` — restore the startup value.
    Default,
    /// A single scalar value.
    To(String),
}

/// The value a `SET <name> {TO|=} <value>` assigns, for the settings
/// [`SessionSettings`] records.
///
/// `None` when the shape is not a single-value assignment at all, which leaves the
/// setting untouched rather than guessing.
fn set_statement_value(value: &[Token], sql: &str) -> Option<SetValue> {
    let (assign, items) = value.split_first()?;
    if !matches!(assign.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq) {
        return None;
    }
    let [item] = items else {
        return None;
    };
    if matches!(item.kind, TokenKind::Keyword(Keyword::Default)) {
        return Some(SetValue::Default);
    }
    // A value position: a string literal carries its content, and a word carries
    // itself. `SET application_name TO foo` and `… TO 'foo'` are the same value on
    // 18.6, measured.
    match item.kind {
        TokenKind::String
        | TokenKind::QuotedIdentifier
        | TokenKind::Identifier
        | TokenKind::Keyword(_) => Some(SetValue::To(item.text(sql).to_string())),
        TokenKind::Number => Some(SetValue::To(numeric_only_text(false, item.text(sql)))),
        _ => None,
    }
}

/// Refuse `READ ONLY` among a `SET`'s transaction-mode words.
///
/// The words are scanned rather than parsed: the modes are unordered and this side
/// decides one question. Every other mode (`ISOLATION LEVEL …`, `READ WRITE`,
/// `[NOT] DEFERRABLE`) stays the accepted no-op it is today.
///
/// # Errors
///
/// `0A000` when the words contain the adjacent pair `READ ONLY`.
fn check_transaction_mode_words(words: &[Token], sql: &str) -> Result<()> {
    for pair in words.windows(2) {
        if is_word(&pair[0], sql, "read") && is_word(&pair[1], sql, "only") {
            return Err(Error::NotImplemented(
                "READ ONLY transactions are not supported".into(),
            ));
        }
    }
    Ok(())
}

/// **The one decider for a GUC whose value pglet cannot honour, whatever wrote it.**
///
/// The rule the `search_path` gate established, generalised to the second parameter
/// that needs it: a value the engine will not act on must be refused, not echoed.
/// Both routes reach this — the `SET` statement (through `check_set_command`) and
/// `set_config(<name>, <value>, …)` (through `planner::expr`'s arm) — because a
/// GUC's value is a property of the *request*, not of the statement that spelled it.
///
/// `value` is `None` for `SET <name> TO DEFAULT` and for a `RESET`, which restore a
/// value pglet does model and are always accepted.
///
/// `client_encoding`, `quote_all_identifiers` and the parameters whose value pglet holds fixed
/// ([`check_fixed_guc`]: `TimeZone`, `DateStyle`, `IntervalStyle`,
/// `standard_conforming_strings`) are here; `search_path` has its own list-shaped decider
/// ([`check_search_path_elements`]). 18.6's own verdict on the value
/// ([`check_guc_value_postgres`]) is asked first, so a value it refuses gets 18.6's answer
/// ahead of pglet's refusal; that function says which of 18.6's checks it models.
///
/// # Errors
///
/// 18.6's own `22023` for a value [`check_guc_value_postgres`] refuses; `0A000` for one pglet
/// cannot honour.
pub fn check_guc_value(name: &str, value: Option<&str>) -> Result<()> {
    check_guc_value_postgres(name, value)?;
    if name == "client_encoding" {
        return check_client_encoding(value);
    }
    if name == "quote_all_identifiers" {
        return check_quote_all_identifiers(value);
    }
    check_fixed_guc(name, value)
}

/// Refuse moving a parameter whose value pglet holds fixed.
///
/// pglet renders and reads every temporal value in UTC and in ISO, month-day order, an interval
/// in the `postgres` style, and a string literal with `standard_conforming_strings` on; it has
/// no other behavior to switch to. 18.6 takes the new value and acts on it — after `SET TIME
/// ZONE 'America/New_York'`, `'2024-01-01 03:00+00'::timestamptz` renders `2023-12-31
/// 22:00:00-05` and casts to the `date` `2023-12-31` (measured) —
/// so accepting the statement left the session answering as though it had not been sent. The
/// value pglet already has is accepted, in spellings 18.6 reads as it and after which 18.6's
/// `SHOW` answers what pglet's does, each matched case-insensitively:
///
/// - `TimeZone`: `UTC`. A zero-offset zone under another name is refused (`GMT`, `Etc/UTC`,
///   `0`): 18.6's `SHOW` and `ParameterStatus` answer each under its own name, and pglet's
///   answer `UTC`.
/// - `DateStyle`: `ISO, MDY`, `ISO,MDY`, `ISO`, `MDY`, and the empty value — 18.6 shows `ISO,
///   MDY` after each.
/// - `IntervalStyle`: `postgres`.
/// - `standard_conforming_strings`: a Boolean that is on.
///
/// That is narrower than what 18.6 reads as the same value (`DateStyle`'s `MDY, ISO`, `US`,
/// `NonEuro…` and `DEFAULT` words, a quoted list element, whitespace around a comma), and the
/// rest is refused rather than read. A value 18.6 itself refuses `22023` — a zone it does not
/// know, an unrecognized `DateStyle` word — takes this function's code too, pglet having no
/// list of zones to tell the two apart; `standard_conforming_strings`' non-Boolean is the
/// exception ([`check_guc_value_postgres`]). `EDGES.md` row 69 holds the measured cells.
///
/// # Errors
///
/// `0A000` naming the parameter, the value and the value pglet has.
fn check_fixed_guc(name: &str, value: Option<&str>) -> Result<()> {
    let (Some(value), Some(fixed)) = (value, fixed_guc_value(name)) else {
        return Ok(());
    };
    let honored = if name == "standard_conforming_strings" {
        parse_guc_bool(value) == Some(true)
    } else {
        let more: &[&str] = if name == "datestyle" {
            &["", "iso", "mdy", "iso,mdy"]
        } else {
            &[]
        };
        value.eq_ignore_ascii_case(fixed) || more.iter().any(|s| value.eq_ignore_ascii_case(s))
    };
    if honored {
        return Ok(());
    }
    Err(Error::NotImplemented(format!(
        "{name} \"{value}\" is not supported: pglet's is always \"{fixed}\""
    )))
}

/// The one value of a parameter [`check_fixed_guc`] holds fixed — what `SHOW` answers for it,
/// and what a `set_config` that names it returns — or `None` for any other parameter. `name`
/// is lower-cased.
#[must_use]
pub fn fixed_guc_value(name: &str) -> Option<&'static str> {
    Some(match name {
        "timezone" => FIXED_TIME_ZONE,
        "datestyle" => FIXED_DATE_STYLE,
        "intervalstyle" => FIXED_INTERVAL_STYLE,
        "standard_conforming_strings" => "on",
        _ => return None,
    })
}

const FIXED_TIME_ZONE: &str = "UTC";
const FIXED_DATE_STYLE: &str = "ISO, MDY";
const FIXED_INTERVAL_STYLE: &str = "postgres";

/// Refuse turning `quote_all_identifiers` on.
///
/// 18.6's `quote_identifier` reads it, so with it on every identifier it renders comes back
/// quoted — `quote_ident('abc')` is `"abc"`, `pg_get_serial_sequence`'s answer and
/// `pg_get_constraintdef`'s `CHECK (("a" > 0))` too (measured). pglet renders names in several
/// readers and honors the setting in none, so accepting it (the ordinary GUC echo, `EDGES.md`
/// row 67) left each of them answering the unquoted spelling with no error. `off`, a reset and
/// `DEFAULT` are what pglet does, and pass; a value that is not a Boolean is 18.6's own `22023`,
/// raised before this by [`check_guc_value_postgres`] through `parse_guc_bool`, 18.6's
/// `parse_bool` grammar — case-insensitive, `1`/`0`, and any unambiguous prefix of
/// `true`/`false`/`yes`/`no`/`on`/`off`, nothing trimmed (`'tr'` is on, `'of'` off, `'o'` and
/// `' on'` refused, measured).
///
/// # Errors
///
/// `0A000` for a value that turns it on.
fn check_quote_all_identifiers(value: Option<&str>) -> Result<()> {
    if value.and_then(parse_guc_bool) == Some(true) {
        return Err(Error::NotImplemented(
            "quote_all_identifiers = on: pglet does not quote every identifier it renders".into(),
        ));
    }
    Ok(())
}

/// A Boolean setting's value as 18.6's `parse_bool` reads it, `None` for one it refuses.
fn parse_guc_bool(value: &str) -> Option<bool> {
    let v = value.to_ascii_lowercase();
    let prefix_of = |word: &str, min: usize| v.len() >= min && word.starts_with(v.as_str());
    if v == "1" || prefix_of("true", 1) || prefix_of("yes", 1) || prefix_of("on", 2) {
        Some(true)
    } else if v == "0" || prefix_of("false", 1) || prefix_of("no", 1) || prefix_of("off", 2) {
        Some(false)
    } else {
        None
    }
}

/// Every name 18.6's `pg_char_to_encoding` knows (`src/common/encnames.c`'s `pg_encname_tbl`),
/// as its `clean_encoding_name` reads one — ASCII letters and digits, lower-cased — which is
/// every name a client encoding may take there.
const POSTGRES_ENCODING_NAMES: &[&str] = &[
    "abc",
    "alt",
    "big5",
    "euccn",
    "eucjis2004",
    "eucjp",
    "euckr",
    "euctw",
    "gb18030",
    "gbk",
    "iso88591",
    "iso885910",
    "iso885913",
    "iso885914",
    "iso885915",
    "iso885916",
    "iso88592",
    "iso88593",
    "iso88594",
    "iso88595",
    "iso88596",
    "iso88597",
    "iso88598",
    "iso88599",
    "johab",
    "koi8",
    "koi8r",
    "koi8u",
    "latin1",
    "latin10",
    "latin2",
    "latin3",
    "latin4",
    "latin5",
    "latin6",
    "latin7",
    "latin8",
    "latin9",
    "mskanji",
    "muleinternal",
    "shiftjis",
    "shiftjis2004",
    "sjis",
    "sqlascii",
    "tcvn",
    "tcvn5712",
    "uhc",
    "unicode",
    "utf8",
    "vscii",
    "win",
    "win1250",
    "win1251",
    "win1252",
    "win1253",
    "win1254",
    "win1255",
    "win1256",
    "win1257",
    "win1258",
    "win866",
    "win874",
    "win932",
    "win936",
    "win949",
    "win950",
    "windows1250",
    "windows1251",
    "windows1252",
    "windows1253",
    "windows1254",
    "windows1255",
    "windows1256",
    "windows1257",
    "windows1258",
    "windows866",
    "windows874",
    "windows932",
    "windows936",
    "windows949",
    "windows950",
];

/// A `GUC_IS_NAME` setting's value as 18.6 stores it, truncated to 63 bytes.
///
/// `parse_and_validate_value` (`utils/misc/guc.c`) truncates it with `truncate_identifier` before the check hook runs —
/// to `NAMEDATALEN - 1` (63) bytes, never splitting a character — so `client_encoding`'s
/// check sees at most 63 bytes (measured on 18.6: `'utf8'` then 60 spaces is `UTF8`, 59
/// dashes then `'utf8'` is `22023 … "…utf"`, with a `NOTICE 42622` pglet does not send).
/// `pg_char_to_encoding`'s own refusal of a name of 64 bytes or more is therefore never reached
/// from a setting.
fn truncate_name_value(value: &str) -> &str {
    const MAX_NAME_BYTES: usize = 63;
    if value.len() <= MAX_NAME_BYTES {
        return value;
    }
    let mut end = MAX_NAME_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// A `client_encoding` value as 18.6's lookup reads it.
///
/// Truncated as a `GUC_IS_NAME` value is (`truncate_name_value`), then cleaned
/// (`clean_encoding_name`) — `"muleinternal"` for every spelling of `MULE_INTERNAL`.
#[must_use]
pub fn client_encoding_key(value: &str) -> String {
    clean_encoding_name(truncate_name_value(value))
}

/// An encoding name as 18.6's `clean_encoding_name` reads it: its ASCII letters and digits,
/// lower-cased, everything else dropped.
fn clean_encoding_name(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// 18.6's **own** verdict on a value for a setting pglet decides.
///
/// It is the check 18.6's GUC machinery makes before anything pglet adds: a `client_encoding` must
/// be a name `pg_char_to_encoding` knows (measured: every name in `POSTGRES_ENCODING_NAMES` but
/// `mule_internal` is `SET` on 18.6, which refuses that one `0A000` as pglet does; `mule` and the
/// empty string are `22023`), a `quote_all_identifiers` or a `standard_conforming_strings` a
/// Boolean `parse_bool` reads. 18.6's checks of `TimeZone`, `DateStyle` and `IntervalStyle` are
/// not modelled: a value it refuses there takes pglet's refusal instead. It is split from pglet's refusal because the two run at different moments
/// where a value can be overridden: at startup 18.6 checks each assignment as it applies it, while
/// pglet's refusal stands in for no Postgres error and so is judged only on the value the session
/// ends with.
///
/// # Errors
///
/// `22023`, worded as 18.6 words it.
pub fn check_guc_value_postgres(name: &str, value: Option<&str>) -> Result<()> {
    let Some(value) = value else { return Ok(()) };
    let value = if name == "client_encoding" {
        truncate_name_value(value)
    } else {
        value
    };
    let refused = match name {
        "client_encoding" => {
            !POSTGRES_ENCODING_NAMES.contains(&clean_encoding_name(value).as_str())
        }
        "quote_all_identifiers" | "standard_conforming_strings" => parse_guc_bool(value).is_none(),
        _ => false,
    };
    if !refused {
        return Ok(());
    }
    let message = if name == "client_encoding" {
        format!("invalid value for parameter \"{name}\": \"{value}\"")
    } else {
        format!("parameter \"{name}\" requires a Boolean value")
    };
    Err(Error::InvalidParameterValue {
        message,
        hint: None,
    })
}

/// Whether [`check_guc_value`] decides this GUC's value — i.e. whether a value the
/// reader cannot see must be refused rather than echoed.
///
/// `search_path` is not here: it has its own decider and its own caller-side branch.
#[must_use]
pub fn guc_value_is_checked(name: &str) -> bool {
    matches!(name, "client_encoding" | "quote_all_identifiers") || fixed_guc_value(name).is_some()
}

/// Refuse a `client_encoding` that is not UTF-8.
///
/// pglet is UTF-8 end to end — it has no encoding-conversion layer at all — and it
/// both asserts `client_encoding=UTF8` in its startup packet and reads the packet's
/// own `client_encoding` through this decider. Accepting `SET client_encoding TO
/// 'LATIN1'` and then sending UTF-8 bytes is the one modelled-parameter divergence
/// that **corrupts data**: the client decodes what it was told it would get (measured
/// over a startup packet asking for `LATIN1` — 18.6 sends `63 61 66 e9` for `café`
/// and pglet sends `63 61 66 c3 a9`, which that client renders `cafÃ©`). 18.6 accepts
/// and converts, and reports the new value with a `ParameterStatus`; pglet can do
/// neither, so it refuses, which is the same call the `search_path` gate makes for
/// the same reason.
///
/// **`SQL_ASCII` is accepted**, and it is not a UTF-8 spelling: it is Postgres's "no
/// conversion — the bytes pass through", which is exactly what pglet does, so it is the
/// one member of the otherwise-refused set whose refusal would buy nothing. It also has
/// a consumer: `pg_dump` of a `SQL_ASCII` database writes
/// `SET client_encoding = 'SQL_ASCII';` into its preamble, and restoring a plain-text
/// dump is a gate (`test:pg-dump`) and a README claim.
///
/// Postgres's encoding-name match reads only a name's ASCII letters and digits, case
/// folded (`clean_encoding_name`), so the normalization here is that rule rather than a list
/// of spellings: `UTF8`, `UTF-8`, `utf8`, `UNICODE`, `SQL_ASCII`, `sql_ascii`, `sqlascii`
/// and `SQL-ASCII` are each accepted on 18.6 — measured, all eight — as is `' utf 8 '`.
///
/// What `SHOW client_encoding` answers after an accepted `SET` is **`UTF8`, always**.
/// 18.6 echoes what was set; pglet's is a fixed modelled value and
/// reporting `UTF8` is the true statement about the bytes it emits — it emits UTF-8,
/// and "no conversion" from a UTF-8 server *is* UTF-8, so a `SQL_ASCII` client is told
/// something narrower than it asked for rather than something false.
///
/// # Errors
///
/// `0A000` naming the refused encoding.
fn check_client_encoding(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if matches!(
        clean_encoding_name(truncate_name_value(value)).as_str(),
        "utf8" | "unicode" | "sqlascii"
    ) {
        return Ok(());
    }
    Err(Error::NotImplemented(format!(
        "client_encoding \"{value}\" is not supported: pglet is UTF8-only"
    )))
}

/// **The one decider for `search_path`, whatever wrote it.**
///
/// The gate this replaced hung off the *statement classifier*, so it was reached
/// only from `SET`. It is a property of the **request**, not of its spelling:
/// `SELECT pg_catalog.set_config('search_path', 'nosuch', false)` is the same
/// request — it is the line modern `pg_dump` emits, and the CVE-2018-1058 hardening
/// idiom — and it walked straight past. Both routes now extract elements with their
/// own reader (they must: one reads lexer tokens, the other splits a GUC string,
/// and 18.6's own two readers differ the same way — `SET search_path = 'a, b'`
/// stores one schema literally named `a, b` while
/// `set_config('search_path', 'a, b', false)` stores two) and hand them here.
///
/// The rule: **no element may name a schema pglet does not have.** `public` in any
/// spelling and the [`IGNORABLE_SEARCH_PATH_ELEMENTS`] pass; anything else is
/// refused. 18.6 accepts every one of these — a schema may be created later, and a
/// missing one is skipped at resolution — so the refusal is deliberately
/// pglet-stricter, and it is the right answer for an engine with one schema:
/// accepting `search_path = test_1` would leave every later statement reading and
/// writing `public` while the session believed it was isolated.
///
/// **The empty list is accepted**, and that is a decision rather than an oversight.
/// `SELECT pg_catalog.set_config('search_path', '', false);` is `pg_dump`'s own
/// preamble line, and `test:pg-dump` is a gate: refusing it would refuse every
/// modern plain-text dump. It costs a residual — on 18.6 an empty `search_path`
/// makes every *unqualified* name `42P01`, where pglet still resolves it to
/// `public` — which is silent, is `EDGES.md`'s, and is invisible to the dump that
/// motivates it, since `pg_dump` schema-qualifies every name it writes. A list of
/// [`IGNORABLE_SEARCH_PATH_ELEMENTS`] alone (`pg_catalog`, `"$user"`) resolves
/// nothing unqualified on 18.6 for the same reason and is the same residual.
///
/// # Errors
///
/// `0A000` naming the refused list.
fn check_search_path_elements(elements: &[String], requested: &str) -> Result<()> {
    if elements.iter().all(|name| {
        // An **empty** element names no schema at all, and is the other spelling of
        // the empty list: `SET search_path TO ''` stores `""` on 18.6 (measured) and
        // makes every unqualified name `42P01` there, exactly as `set_config(…, '',
        // …)` does. Same acceptance, same residual.
        name.is_empty()
            || name == ONLY_SCHEMA
            || IGNORABLE_SEARCH_PATH_ELEMENTS.contains(&name.as_str())
    }) {
        return Ok(());
    }
    Err(Error::UnsupportedSearchPath {
        requested: requested.to_string(),
    })
}

/// The `SET search_path {TO|=} <list>` reader — tokens to elements, then
/// [`check_search_path_elements`].
///
/// # Errors
///
/// `0A000` for a refused list, and for one this reader cannot parse: a value it
/// cannot see is a value it must not silently accept.
fn check_search_path_statement(value: &[Token], sql: &str) -> Result<()> {
    // The refused list, rendered as written — the `TO`/`=` that introduced it is not
    // part of it.
    let render = |items: &[Token]| {
        items
            .iter()
            .map(|t| t.text(sql))
            .collect::<Vec<_>>()
            .join(" ")
    };
    // `TO` or `=`; nothing else introduces the value.
    let Some((assign, items)) = value.split_first() else {
        return Err(Error::UnsupportedSearchPath {
            requested: render(value),
        });
    };
    if !matches!(assign.kind, TokenKind::Keyword(Keyword::To) | TokenKind::Eq) {
        return Err(Error::UnsupportedSearchPath {
            requested: render(value),
        });
    }
    // `SET search_path TO DEFAULT` restores `"$user", public`, which resolves here
    // exactly as the explicit spelling does.
    if let [only] = items
        && matches!(only.kind, TokenKind::Keyword(Keyword::Default))
    {
        return Ok(());
    }
    let mut names = Vec::new();
    for group in items.split(|t| matches!(t.kind, TokenKind::Comma)) {
        let [item] = group else {
            return Err(Error::UnsupportedSearchPath {
                requested: render(items),
            });
        };
        match item.kind {
            // An identifier folds, a `"…"`-quoted one is verbatim — `Token::name`'s
            // rule, the same one the parser reads a name under.
            TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_) => {
                names.push(item.name(sql));
            }
            // A string-valued element is **one** element on this route: measured on
            // 18.6, `SET search_path = 'public, foo'` sets the single schema
            // literally named `public, foo` (`SHOW` renders it quoted), because
            // `flatten_set_variable_args` quotes each string element before the list
            // is stored. The `set_config` route stores the string raw, and there the
            // same text is two elements — which is why the two readers are separate
            // and only the decider is shared.
            TokenKind::String => names.push(fold_search_path_string(item.text(sql))),
            _ => {
                return Err(Error::UnsupportedSearchPath {
                    requested: render(items),
                });
            }
        }
    }
    check_search_path_elements(&names, &render(items))
}

/// The `set_config('search_path', <value>, <is_local>)` reader — a GUC value string
/// to elements, then [`check_search_path_elements`].
///
/// The split is `SplitIdentifierString`'s: commas outside `"…"` separate, a quoted
/// part is verbatim, an unquoted one is ASCII-downcased, and surrounding whitespace
/// is dropped. `is_local` does not enter the decision — 18.6 applies the same
/// `check_search_path` hook to both, and a `true` value only changes how long the
/// setting lives.
///
/// # Errors
///
/// `0A000` for a refused list.
pub fn check_search_path_value(value: &str) -> Result<()> {
    check_search_path_elements(&split_guc_list(value), value)
}

/// Postgres's `SplitIdentifierString` over a `GUC_LIST_INPUT` value: split on
/// commas that are not inside `"…"`, then fold each element.
fn split_guc_list(value: &str) -> Vec<String> {
    let mut elements = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in value.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                elements.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    elements.push(current);
    // A single empty element is the **empty list**, not a list holding `""`: an
    // unset `search_path` is `''`, and 18.6 reads it as naming no schema at all.
    if elements.len() == 1 && elements[0].trim().is_empty() {
        return Vec::new();
    }
    elements
        .iter()
        .map(|e| fold_search_path_string(e))
        .collect()
}

/// One `search_path` element under Postgres's `SplitIdentifierString` rule: a
/// `"…"`-quoted part verbatim, anything else ASCII-downcased. `'$user'` and
/// `'"$user"'` therefore both name `$user`.
fn fold_search_path_string(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed[1..trimmed.len() - 1].to_string();
    }
    trimmed.to_ascii_lowercase()
}

/// A runtime parameter pglet recognizes for `SHOW`.
///
/// Parsing the variable name into this closed set (rather than `.contains()`
/// substring tests over the SQL) makes the value table one exhaustive `match`
/// and turns an unknown name into a loud `UnrecognizedParameter` error matching
/// Postgres — not a silently-empty result, and not a false match on a name that
/// merely *contains* a known one.
///
/// It is also the **one home** for each value. The startup `ParameterStatus` block
/// the native server sends reports a subset of these, and it used to type its own
/// copies: `server_version` was `16.0` in both places, and the day one moved the
/// other would not have. [`Self::STARTUP_REPORTED`] names that subset and
/// [`startup_parameters`] renders it, so the two cannot disagree.
enum ShowVariable {
    ServerVersion,
    ServerVersionNum,
    SearchPath,
    TransactionIsolation,
    DefaultTransactionIsolation,
    ServerEncoding,
    ClientEncoding,
    ApplicationName,
    DateStyle,
    IntervalStyle,
    TimeZone,
    StandardConformingStrings,
    IntegerDatetimes,
    LockTimeout,
}

impl ShowVariable {
    /// Every parameter pglet models — the rows `SHOW ALL` reports. [`Self::value`]
    /// stays the single source of truth for each one's value.
    const ALL: [Self; 14] = [
        Self::ServerVersion,
        Self::ServerVersionNum,
        Self::SearchPath,
        Self::TransactionIsolation,
        Self::DefaultTransactionIsolation,
        Self::ServerEncoding,
        Self::ClientEncoding,
        Self::ApplicationName,
        Self::DateStyle,
        Self::IntervalStyle,
        Self::TimeZone,
        Self::StandardConformingStrings,
        Self::IntegerDatetimes,
        Self::LockTimeout,
    ];

    /// The subset the native server reports as `ParameterStatus` — at startup, and
    /// again whenever one of their values **changes** (see
    /// [`execute_session_command`]'s callers).
    ///
    /// This is `GUC_REPORT`, and the membership is measured rather than reasoned:
    /// 18.6's startup packet carries `in_hot_standby`, `integer_datetimes`,
    /// `TimeZone`, `IntervalStyle`, `search_path`, `is_superuser`,
    /// `application_name`, `default_transaction_read_only`, `scram_iterations`,
    /// `DateStyle`, `standard_conforming_strings`, `session_authorization`,
    /// `client_encoding`, `server_version` and `server_encoding`. The six pglet
    /// omits (`in_hot_standby`, `is_superuser`, `default_transaction_read_only`,
    /// `scram_iterations`, `session_authorization`, and `transaction_isolation`
    /// — which is not `GUC_REPORT` on either engine) are ones it models no value
    /// for; `server_version_num` is likewise not reported by 18.6.
    ///
    /// **`search_path` is in the list, and the comment that said it was not was
    /// backwards.** It has carried `GUC_REPORT` since PostgreSQL 14, and 18.6 sends
    /// it at connect and after every `SET`/`set_config` that moves it — measured
    /// with a raw v3 client, both directions.
    const STARTUP_REPORTED: [Self; 10] = [
        Self::ServerVersion,
        Self::ServerEncoding,
        Self::ClientEncoding,
        Self::ApplicationName,
        Self::SearchPath,
        Self::DateStyle,
        Self::IntervalStyle,
        Self::TimeZone,
        Self::StandardConformingStrings,
        Self::IntegerDatetimes,
    ];

    /// Match a normalized parameter name, or `None` if pglet doesn't model it.
    fn parse(name: &str) -> Option<Self> {
        match name {
            "server_version" => Some(Self::ServerVersion),
            "server_version_num" => Some(Self::ServerVersionNum),
            "search_path" => Some(Self::SearchPath),
            // `SHOW transaction_isolation` and `SHOW TRANSACTION ISOLATION [LEVEL]`
            // both name the isolation setting.
            "transaction_isolation" | "transaction isolation" | "transaction isolation level" => {
                Some(Self::TransactionIsolation)
            }
            "default_transaction_isolation" => Some(Self::DefaultTransactionIsolation),
            "server_encoding" => Some(Self::ServerEncoding),
            "client_encoding" => Some(Self::ClientEncoding),
            "application_name" => Some(Self::ApplicationName),
            "datestyle" => Some(Self::DateStyle),
            "intervalstyle" => Some(Self::IntervalStyle),
            // `SHOW TIME ZONE` is the same setting as `SHOW TimeZone`, and 18.6
            // answers both under the column name `TimeZone`.
            "timezone" | "time zone" => Some(Self::TimeZone),
            "standard_conforming_strings" => Some(Self::StandardConformingStrings),
            "integer_datetimes" => Some(Self::IntegerDatetimes),
            "lock_timeout" => Some(Self::LockTimeout),
            _ => None,
        }
    }

    /// The reported column name — the GUC's **canonical** spelling, which is what
    /// 18.6 answers under regardless of how the statement spelled it (`SHOW
    /// datestyle` and `SHOW TIME ZONE` report `DateStyle` and `TimeZone`).
    const fn name(&self) -> &'static str {
        match self {
            Self::ServerVersion => "server_version",
            Self::ServerVersionNum => "server_version_num",
            Self::SearchPath => "search_path",
            Self::TransactionIsolation => "transaction_isolation",
            Self::DefaultTransactionIsolation => "default_transaction_isolation",
            Self::ServerEncoding => "server_encoding",
            Self::ClientEncoding => "client_encoding",
            Self::ApplicationName => "application_name",
            Self::DateStyle => "DateStyle",
            Self::IntervalStyle => "IntervalStyle",
            Self::TimeZone => "TimeZone",
            Self::StandardConformingStrings => "standard_conforming_strings",
            Self::IntegerDatetimes => "integer_datetimes",
            Self::LockTimeout => "lock_timeout",
        }
    }

    /// This parameter's value for `settings`' session.
    ///
    /// Most are facts about the build that no statement moves — pglet is UTC-only,
    /// UTF8-only, integer-datetimes-on — so a fixed string is the right answer.
    /// `application_name`, `lock_timeout`, `transaction_isolation` and
    /// `default_transaction_isolation` are the session's ([`SessionSettings`]).
    ///
    /// `server_version` reports the release pglet's differential corpus is asserted
    /// against, and `server_version_num` the same release as the number a client
    /// feature-gates on. It names the **dialect target**, not feature completeness:
    /// pglet implements a subset of that release's SQL, and
    /// `docs/compatibility.md` is what says which.
    ///
    /// `search_path` reports [`DEFAULT_SEARCH_PATH`] and **does not move**: an
    /// accepted `SET`/`set_config` cannot name a schema pglet lacks
    /// ([`check_search_path_elements`]), so every accepted value resolves the same
    /// way, and pglet keeps no session cell for the string. What that costs — the
    /// echo, and the `ParameterStatus` 18.6 sends after an accepted list that reads
    /// differently — is `EDGES.md`'s.
    fn value<'a>(&self, settings: &'a SessionSettings) -> Cow<'a, str> {
        Cow::Borrowed(match self {
            Self::ServerVersion => SERVER_VERSION,
            Self::ServerVersionNum => SERVER_VERSION_NUM,
            Self::SearchPath => DEFAULT_SEARCH_PATH,
            Self::TransactionIsolation => settings.transaction_isolation().name(),
            Self::DefaultTransactionIsolation => settings.default_isolation().name(),
            Self::ServerEncoding | Self::ClientEncoding => "UTF8",
            Self::ApplicationName => settings.application_name(),
            Self::DateStyle => FIXED_DATE_STYLE,
            Self::IntervalStyle => FIXED_INTERVAL_STYLE,
            Self::TimeZone => FIXED_TIME_ZONE,
            Self::StandardConformingStrings | Self::IntegerDatetimes => "on",
            Self::LockTimeout => {
                return Cow::Owned(format_lock_timeout(settings.lock_timeout_ms()));
            }
        })
    }
}

/// The runtime parameters a host reports as `ParameterStatus`, as `(name, value)`
/// pairs, in the order the startup packet sends them.
///
/// Read twice by the native server: once to build the startup block, and once
/// before/after each session command so a **change** is reported, which is
/// Postgres's rule (`GUC_REPORT` is sent at connect and then only when the value
/// moves — measured: a second identical `SET search_path TO public` sends nothing,
/// and `SET client_encoding TO 'UTF8'` on a UTF8 connection sends nothing).
///
/// Most of these are fixed values pglet can truthfully report — a client keying off
/// `standard_conforming_strings` for literal escaping, `integer_datetimes` for the
/// binary timestamp format, or `DateStyle` for date parsing no longer has to guess.
/// `application_name` is the session's, so this takes [`SessionSettings`] rather
/// than being a constant.
#[must_use]
pub fn startup_parameters(settings: &SessionSettings) -> Vec<(&'static str, Cow<'_, str>)> {
    ShowVariable::STARTUP_REPORTED
        .iter()
        .map(|var| (var.name(), var.value(settings)))
        .collect()
}

/// Extract the normalized parameter name from a `SHOW <name>` statement: the text
/// after the leading `SHOW`, lowercased, trailing `;` dropped, internal
/// whitespace collapsed (so `SHOW   TRANSACTION  ISOLATION` → `transaction
/// isolation`).
fn parse_show_name(sql: &str) -> String {
    let trimmed = trim_space(trim_space(sql).trim_end_matches(';'));
    // `is_show_command` matched an **ASCII**-folded `SHOW` and a byte of the lexer's
    // whitespace after it, so the first four characters are one byte each and `4..`
    // lands on the boundary after them. A wider fold there breaks that: `ſ` is two
    // bytes, and `ſHOW search_path` reached here as `"W search_path"`.
    let rest = trimmed.get(4..).unwrap_or("");
    // A quoted name (`SHOW "Search_Path"`) is the same setting, case-insensitively, as on 18.6;
    // no setting's name holds a `"`, so dropping them is only the unquoting.
    rest.split(|c: char| u8::try_from(c).is_ok_and(is_space))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .replace('"', "")
}

/// Handle SHOW commands.
///
/// # Errors
///
/// Returns `UnrecognizedParameter` (Postgres SQLSTATE `42704`) for a parameter
/// pglet doesn't model.
pub fn execute_show_command(sql: &str, settings: &SessionSettings) -> Result<QueryResult> {
    let name = parse_show_name(sql);
    if name == "all" {
        // `SHOW ALL` lists every modeled parameter as `name | setting | description`
        // (Postgres's column shape). pglet models a small subset and leaves the
        // description blank; an unmodeled parameter is simply absent rather than an
        // error — `SHOW ALL` never claims Postgres's full ~350-setting catalog.
        let text = |s: &str| ColumnDesc::new(s, TypeOid::Text);
        return Ok(QueryResult {
            shape: ResultShape::Columns(vec![text("name"), text("setting"), text("description")]),
            rows: ShowVariable::ALL
                .iter()
                .map(|var| {
                    vec![
                        Value::Text(var.name().into()),
                        Value::Text(var.value(settings).into()),
                        Value::Text(String::new()),
                    ]
                })
                .collect(),
            command_tag: "SHOW".into(),
            notices: Vec::new(),
            tx_command: None,
            swapped_sequences: Vec::new(),
        });
    }
    let Some(var) = ShowVariable::parse(&name) else {
        return Err(Error::UnrecognizedParameter(name));
    };
    Ok(QueryResult {
        shape: ResultShape::Columns(vec![ColumnDesc::new(var.name(), TypeOid::Text)]),
        rows: vec![vec![Value::Text(var.value(settings).into())]],
        command_tag: "SHOW".into(),
        notices: Vec::new(),
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

#[cfg(test)]
mod fold_width_tests {
    //! The classifiers fold at the lexer's ASCII width, because they decide *ahead*
    //! of the parser — see the module doc for the measured pairs.

    use super::{
        SessionSettings, execute_show_command, is_set_command, is_show_command, parse_show_name,
    };

    #[test]
    fn a_long_s_does_not_classify_as_set_or_show() {
        // U+017F uppercases to ASCII `S`, so a Unicode fold matched both prefixes:
        // `ſET …` ran here as a `SET` and `ſHOW …` answered `42704`, where 18.6
        // answers `42601` for both.
        assert!(!is_set_command("ſET client_min_messages = 'warning'"));
        assert!(!is_show_command("ſHOW search_path"));
        // The accept side: an ASCII spelling in any case still classifies.
        assert!(is_set_command("  set client_min_messages = 'warning'  "));
        assert!(is_show_command("Show search_path"));
    }

    #[test]
    fn an_unknown_parameter_name_folds_ascii_only() {
        // 18.6's scanner downcases ASCII, so it reports `fÖÖ`; a Unicode fold
        // reported `föö` and an `İ` grew a combining dot.
        let err =
            execute_show_command("SHOW FÖÖ", &SessionSettings::default()).expect_err("unknown");
        assert_eq!(
            err.to_string(),
            r#"unrecognized configuration parameter "fÖÖ""#
        );
        assert_eq!(parse_show_name("SHOW İSOLATION"), "İsolation");
    }
}

#[cfg(test)]
mod show_tests {
    use super::{SessionSettings, execute_show_command, parse_show_name};

    #[test]
    fn parses_name_case_insensitively_and_normalizes_whitespace() {
        assert_eq!(parse_show_name("SHOW server_version"), "server_version");
        assert_eq!(parse_show_name("show Search_Path;"), "search_path");
        assert_eq!(
            parse_show_name("SHOW   TRANSACTION  ISOLATION  LEVEL "),
            "transaction isolation level"
        );
    }

    /// The compile-time relation between the two version literals, both polarities:
    /// a pair that drifted apart, a zero-padded number, a missing minor.
    #[test]
    fn version_num_spells_the_release() {
        use super::version_num_spells;
        assert!(version_num_spells("18.6", "180006"));
        assert!(version_num_spells("9.12", "90012"));
        assert!(!version_num_spells("18.6", "180004"));
        assert!(!version_num_spells("18.4", "180006"));
        assert!(!version_num_spells("18.6", "0180006"));
        assert!(!version_num_spells("18", "180000"));
        assert!(!version_num_spells("18.6.1", "180006"));
        assert!(!version_num_spells("18.6", "18x006"));
    }

    #[test]
    fn recognized_parameters_report_their_value() {
        for (sql, want) in [
            // The release the differential corpus is asserted against. 18.6's own
            // `SHOW server_version` carries a packaging suffix
            // (`18.6 (Debian 18.6-1.pgdg13+2)`) that no client parses, so the
            // leading `major.minor` is what is reported and what is pinned here —
            // this is also why the parameter cannot be a plain differential fixture.
            ("SHOW server_version", "18.6"),
            ("SHOW server_version_num", "180006"),
            // 18.6's own default, byte for byte: `$user` names a schema a fresh
            // database does not have there either, so both engines resolve this list
            // to `public` alone.
            ("SHOW search_path", "\"$user\", public"),
            ("SHOW transaction_isolation", "read committed"),
            ("SHOW TRANSACTION ISOLATION LEVEL", "read committed"),
            // The canonical GUC spelling is the reported column name whatever the
            // statement wrote, measured on 18.6.
            ("SHOW datestyle", "ISO, MDY"),
            ("SHOW TIME ZONE", "UTC"),
            ("SHOW standard_conforming_strings", "on"),
        ] {
            let result =
                execute_show_command(sql, &SessionSettings::default()).expect("recognized");
            assert_eq!(
                result.rows,
                vec![vec![crate::types::Value::Text(want.into())]]
            );
        }
    }

    #[test]
    fn unknown_parameter_errors_loud() {
        // The old `.contains()` dispatch returned an empty result here; now it is a
        // loud `UnrecognizedParameter` (Postgres 42704), and a typo of a known name
        // no longer silently matches nothing.
        let err = execute_show_command("SHOW serch_path", &SessionSettings::default())
            .expect_err("unknown");
        assert_eq!(err.sqlstate(), "42704");
        assert!(matches!(err, crate::Error::UnrecognizedParameter(_)));
    }

    #[test]
    fn the_startup_report_reads_the_same_table_show_does() {
        // The two homes this pair replaced: the startup `ParameterStatus` block
        // typed its own `server_version` value, and it said `16.0` alongside a
        // `SHOW` that said `16.0` — one edit away from a client being told two
        // different releases by the same connection.
        let settings = SessionSettings::default();
        let startup = super::startup_parameters(&settings);
        assert_eq!(
            startup.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            vec![
                "server_version",
                "server_encoding",
                "client_encoding",
                "application_name",
                "search_path",
                "DateStyle",
                "IntervalStyle",
                "TimeZone",
                "standard_conforming_strings",
                "integer_datetimes",
            ]
        );
        for (name, value) in startup {
            let shown = execute_show_command(&format!("SHOW {name}"), &settings).expect("modeled");
            assert_eq!(
                shown.rows,
                vec![vec![crate::types::Value::Text(value.into())]],
                "{name} disagrees between the startup report and SHOW"
            );
        }
    }

    #[test]
    fn show_all_lists_modeled_settings() {
        // `SHOW ALL` is not an unknown parameter — it lists every modeled setting
        // as name/setting/description (one row per modeled parameter).
        let result = execute_show_command("SHOW ALL", &SessionSettings::default())
            .expect("SHOW ALL is recognized");
        assert_eq!(result.columns().len(), 3);
        assert_eq!(result.columns()[0].name, "name");
        assert_eq!(result.columns()[1].name, "setting");
        assert_eq!(result.rows.len(), super::ShowVariable::ALL.len());
        // server_version is the first modeled setting.
        assert_eq!(
            result.rows[0][0],
            crate::types::Value::Text("server_version".into())
        );
        assert_eq!(result.rows[0][1], crate::types::Value::Text("18.6".into()));
    }
}

// ============================================================================
// DEALLOCATE
// ============================================================================

/// What a `DEALLOCATE` names — `gram.y`'s `DeallocateStmt`, whose four spellings
/// (`DEALLOCATE [PREPARE] { name | ALL }`) collapse to these two targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeallocateTarget {
    /// `DEALLOCATE ALL` / `DEALLOCATE PREPARE ALL`.
    All,
    /// `DEALLOCATE [PREPARE] <name>`, already folded as a `ColId`.
    Named(String),
}

/// The session's named prepared statements, as `DEALLOCATE` sees them.
///
/// A trait rather than a concrete map because the three hosts hold this namespace
/// in three different places — the native server on its per-socket `Connection`,
/// the embed facade and the wasm bindings nowhere at all, since neither models a
/// `Parse`/`Bind` handle. A host with no namespace is not a special case to code
/// around: it answers exactly what Postgres answers for a session that has
/// prepared nothing, which is `26000` for a name and success for `ALL`.
pub trait PreparedStatements {
    /// Drop `name` (already folded), answering whether it was there.
    fn remove_prepared(&mut self, name: &str) -> bool;
    /// Drop every **named** statement, leaving the unnamed slot.
    ///
    /// The exemption is load-bearing and is not a pglet convenience: Postgres keeps
    /// the protocol's unnamed statement in a separate `unnamed_stmt_psrc` slot
    /// rather than in `prepared_queries`, so no `DEALLOCATE` can reach it — SQL
    /// cannot even name it, `PREPARE ""` being refused. pglet keys the unnamed slot
    /// `""` in the *same* map, so the rule has to be written down here or a
    /// `DEALLOCATE ALL` would close a portal's statement mid-flight.
    fn clear_named_prepared(&mut self);
}

impl<V, S: std::hash::BuildHasher> PreparedStatements for HashMap<String, V, S> {
    fn remove_prepared(&mut self, name: &str) -> bool {
        self.remove(name).is_some()
    }

    fn clear_named_prepared(&mut self) {
        self.retain(|name, _| name.is_empty());
    }
}

/// A host that holds no named prepared statements — the embed facade and the wasm
/// bindings, which take parameter values directly and have no statement handle.
impl PreparedStatements for () {
    fn remove_prepared(&mut self, _name: &str) -> bool {
        false
    }

    fn clear_named_prepared(&mut self) {}
}

/// Whether `sql`'s first statement is a `DEALLOCATE`.
///
/// `DEALLOCATE` is an UNRESERVED keyword in Postgres and deliberately **not** in
/// pglet's lexer table, so it reaches this classifier as an `Identifier` — which
/// is also what keeps `SELECT deallocate FROM t` and `CREATE TABLE deallocate (…)`
/// working, as they do on 18.6. Nothing else in either grammar begins a statement
/// with a bare word, so the leading token decides.
#[must_use]
pub fn is_deallocate_command(sql: &str) -> bool {
    lead_tokens(sql, 1)
        .first()
        .is_some_and(|t| is_word(t, sql, "deallocate"))
}

/// Parse a `DEALLOCATE` statement into its target.
///
/// Only the leading words are read, and the shape is `gram.y`'s: an optional
/// `PREPARE` noise word, then the reserved `ALL` or a `ColId`. Two spellings are
/// worth stating because they look like the other branch and are not, both
/// measured on 18.6: `DEALLOCATE "ALL"` drops the statement *named* `ALL` (the
/// keyword is reserved, so a quoted spelling can only be a name), and a bare
/// `DEALLOCATE prepare` drops the one named `prepare` (the noise word is consumed
/// only when a target follows it).
///
/// # Errors
///
/// `42601` for a missing or trailing token, and whatever the lexer raises for a
/// malformed name (`42602` for `DEALLOCATE ""`).
pub fn parse_deallocate(sql: &str) -> Result<DeallocateTarget> {
    let tokens = all_tokens(sql).map_err(|e| lex_error_to_error(e, sql))?;
    // The statement terminator is not a trailing token. `split_statements` strips it
    // on the simple-query path, but the extended protocol's `Parse` hands this the
    // client's raw text, which carries it — so a classifier written against the split
    // form alone answers `42601` to the one spelling every driver actually sends.
    let end = tokens
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Semicolon))
        .unwrap_or(tokens.len());
    let mut rest = tokens.get(1..end).unwrap_or_default();
    // `PREPARE` is a noise word only when something follows it; alone it is the
    // target's own name.
    if rest.len() > 1 && rest.first().is_some_and(|t| is_word(t, sql, "prepare")) {
        rest = &rest[1..];
    }
    let Some(target) = rest.first() else {
        // 18.6 reports the terminator when one was written and "at end of input" when
        // the statement simply stopped — measured both ways.
        return Err(unexpected_end(tokens.get(end), sql));
    };
    if rest.len() > 1 {
        return Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", rest[1].text(sql)),
            pglet_core::codepoint_position(sql, rest[1].start),
        ));
    }
    if is_word(target, sql, "all") || matches!(target.kind, TokenKind::Keyword(Keyword::All)) {
        return Ok(DeallocateTarget::All);
    }
    if !matches!(
        target.kind,
        TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_)
    ) {
        return Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", target.text(sql)),
            pglet_core::codepoint_position(sql, target.start),
        ));
    }
    // A zero-length delimited identifier cannot reach here: the **lexer** refuses it,
    // which is where 18.6's scanner refuses it too (`42601`, not `42602` — measured).
    // This function used to hand-roll that check, which is how the engine came to
    // refuse `DEALLOCATE ""` and accept `CREATE TABLE "" (a int)`.
    Ok(DeallocateTarget::Named(target.name(sql)))
}

/// Apply a parsed `DEALLOCATE` to a session's namespace, answering its command tag.
///
/// The three rules that must not differ between hosts live here: `ALL` never
/// errors (Postgres's `DropAllPreparedStatements` walks an empty table happily and
/// reports `DEALLOCATE ALL`), a name the namespace lacks is `26000` — where the
/// protocol's own `Close` on the same store is a silent no-op, the two differing
/// only by `DropPreparedStatement`'s `showError` flag — and the tag distinguishes
/// the two forms.
///
/// # Errors
///
/// `26000` when `target` names a statement the session does not hold.
pub fn execute_deallocate(
    target: &DeallocateTarget,
    prepared: &mut impl PreparedStatements,
) -> Result<&'static str> {
    match target {
        DeallocateTarget::All => {
            prepared.clear_named_prepared();
            Ok("DEALLOCATE ALL")
        }
        DeallocateTarget::Named(name) => {
            if prepared.remove_prepared(name) {
                Ok("DEALLOCATE")
            } else {
                Err(Error::UndefinedPreparedStatement { name: name.clone() })
            }
        }
    }
}

// ============================================================================
// DISCARD / RESET
// ============================================================================

/// What a `DISCARD` names — `gram.y`'s `DiscardStmt`.
///
/// `TEMPORARY` and `TEMP` are one target (18.6 reports command tag `DISCARD TEMP`
/// for both spellings, measured), so the enum has four members for five words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscardTarget {
    /// `DISCARD ALL` — every one of the others, plus `DEALLOCATE ALL` and
    /// `RESET ALL`.
    All,
    /// `DISCARD PLANS` — the cached-plan store.
    Plans,
    /// `DISCARD SEQUENCES` — the session's `currval` cache.
    Sequences,
    /// `DISCARD TEMP` / `DISCARD TEMPORARY` — the session's temporary schema.
    Temp,
}

impl DiscardTarget {
    /// The command tag 18.6 reports for this target.
    const fn tag(self) -> &'static str {
        match self {
            Self::All => "DISCARD ALL",
            Self::Plans => "DISCARD PLANS",
            Self::Sequences => "DISCARD SEQUENCES",
            Self::Temp => "DISCARD TEMP",
        }
    }
}

/// Whether `sql`'s first statement is a `DISCARD`.
///
/// Read off the lexer for the same reason [`is_deallocate_command`] is: `DISCARD`
/// is UNRESERVED in Postgres and deliberately not in pglet's keyword table, so it
/// arrives as an `Identifier` — which is what keeps `SELECT discard FROM t` and
/// `CREATE TABLE discard (…)` working, as they do on 18.6.
#[must_use]
pub fn is_discard_command(sql: &str) -> bool {
    lead_tokens(sql, 1)
        .first()
        .is_some_and(|t| is_word(t, sql, "discard"))
}

/// Whether `sql`'s first statement is a `RESET`. See [`is_discard_command`] for why
/// this reads the lexer.
#[must_use]
pub fn is_reset_command(sql: &str) -> bool {
    lead_tokens(sql, 1)
        .first()
        .is_some_and(|t| is_word(t, sql, "reset"))
}

/// The single leading word a `DISCARD` / `RESET` takes, or the syntax error 18.6
/// raises in its place.
///
/// Both statements have the same shape — the keyword, exactly one word, the end —
/// and the same two failures, so the shape is read once. The terminator is not a
/// trailing token: `split_statements` strips it on the simple-query path, but the
/// extended protocol's `Parse` hands the classifier the client's raw text, which
/// carries it.
fn lead_word_argument<'a>(sql: &'a str, tokens: &'a [Token]) -> Result<&'a Token> {
    let end = tokens
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Semicolon))
        .unwrap_or(tokens.len());
    let rest = tokens.get(1..end).unwrap_or_default();
    let Some(word) = rest.first() else {
        // 18.6 reports the terminator when one was written and "at end of input"
        // when the statement simply stopped — measured both ways.
        return Err(unexpected_end(tokens.get(end), sql));
    };
    if rest.len() > 1 {
        return Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", rest[1].text(sql)),
            pglet_core::codepoint_position(sql, rest[1].start),
        ));
    }
    Ok(word)
}

/// Parse a `DISCARD` statement into its target.
///
/// # Errors
///
/// `42601` for a missing, unknown, or trailing token. The unknown-word message
/// carries the word **as written**: 18.6 answers `DISCARD FOO` with
/// `syntax error at or near "FOO"`, from the scanner, before any fold.
pub fn parse_discard(sql: &str) -> Result<DiscardTarget> {
    // The whole statement: a reader that answers must see the lexer's own error rather
    // than a short read of the tokens before it.
    let tokens = all_tokens(sql).map_err(|e| lex_error_to_error(e, sql))?;
    let word = lead_word_argument(sql, &tokens)?;
    // `ALL` is a reserved keyword and reaches here as one; the rest are ordinary
    // unreserved words the lexer does not carry.
    if matches!(word.kind, TokenKind::Keyword(Keyword::All)) || is_word(word, sql, "all") {
        return Ok(DiscardTarget::All);
    }
    if is_word(word, sql, "plans") {
        return Ok(DiscardTarget::Plans);
    }
    if is_word(word, sql, "sequences") {
        return Ok(DiscardTarget::Sequences);
    }
    if is_word(word, sql, "temp") || is_word(word, sql, "temporary") {
        return Ok(DiscardTarget::Temp);
    }
    Err(Error::syntax_sentence_at(
        format!("syntax error at or near \"{}\"", word.text(sql)),
        pglet_core::codepoint_position(sql, word.start),
    ))
}

/// Parse a `RESET`, answering the GUC name it targets (`None` for `RESET ALL`).
///
/// The grammar is `gram.y`'s `reset_rest`, which is **not** `RESET <one word>`:
/// beside `generic_reset` (a dotted `var_name`, or `ALL`) it has three fixed
/// spellings that name a GUC the words do not spell — `TIME ZONE` → `timezone`,
/// `TRANSACTION ISOLATION LEVEL` → `transaction_isolation`, and `SESSION
/// AUTHORIZATION` → `session_authorization`. All three are ordinary statements on
/// 18.6 (`RESET SESSION AUTHORIZATION` is a pool check-in statement), and reading
/// only one word made each a `42601` here — measured.
///
/// `RESET` is otherwise the GUC family's other spelling and takes the family's
/// posture: every GUC pglet models with a fixed value is one it never moves, so
/// restoring it is a no-op that reports tag `RESET` (18.6 reports `RESET`
/// for `RESET ALL` too, measured — not `RESET ALL`). What is *not* a no-op is the
/// shape check, and the session settings the caller then restores.
///
/// The name is not looked up. 18.6 answers `RESET nosuchguc` with `42704`, and
/// pglet does not, for the same reason `SET nosuchguc = 1` is an accepted no-op
/// here: pglet models a handful of the ~350 settings and refusing every name
/// outside that handful would refuse the real ones too. The pair is `EDGES.md`'s.
///
/// # Errors
///
/// `42601` for a missing or trailing token, or a name token that is not a name.
/// `0A000` for `RESET TRANSACTION ISOLATION LEVEL`, which 18.6 parses and then
/// refuses with `parameter "transaction_isolation" cannot be reset` — measured, and
/// the one `reset_rest` production that is not a success there.
pub fn parse_reset(sql: &str) -> Result<Option<String>> {
    let tokens = all_tokens(sql).map_err(|e| lex_error_to_error(e, sql))?;
    let end = tokens
        .iter()
        .position(|t| matches!(t.kind, TokenKind::Semicolon))
        .unwrap_or(tokens.len());
    let rest = tokens.get(1..end).unwrap_or_default();
    let Some(first) = rest.first() else {
        return Err(unexpected_end(tokens.get(end), sql));
    };
    // The three fixed `reset_rest` spellings, before `generic_reset` — each is more
    // than one word, so a one-word reader saw the second word as a trailing token.
    // Each fixed spelling needs a *following* word to be one: `RESET TIME` alone is
    // `generic_reset` naming the GUC `time` (18.6: `42704`), not a truncated
    // `TIME ZONE`, and the same for `SESSION` and `TRANSACTION`.
    if is_word(first, sql, "time") && rest.len() > 1 {
        expect_reset_word(rest, 1, sql, "zone", tokens.get(end))?;
        return Ok(Some("timezone".into()));
    }
    if is_word(first, sql, "session") && rest.len() > 1 {
        expect_reset_word(rest, 1, sql, "authorization", tokens.get(end))?;
        return Ok(Some("session_authorization".into()));
    }
    if matches!(first.kind, TokenKind::Keyword(Keyword::Transaction)) && rest.len() > 1 {
        expect_reset_word(rest, 1, sql, "isolation", tokens.get(end))?;
        expect_reset_word(rest, 2, sql, "level", tokens.get(end))?;
        return Err(Error::NotImplemented(
            "parameter \"transaction_isolation\" cannot be reset".into(),
        ));
    }
    // `generic_reset`: `ALL`, or a `var_name` — which is dotted (`RESET a.b` is an
    // ordinary statement on 18.6).
    if matches!(first.kind, TokenKind::Keyword(Keyword::All)) || is_word(first, sql, "all") {
        expect_reset_end(rest, 1, sql)?;
        return Ok(None);
    }
    if !matches!(
        first.kind,
        TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_)
    ) {
        return Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", first.text(sql)),
            pglet_core::codepoint_position(sql, first.start),
        ));
    }
    let mut name = first.name(sql);
    let mut i = 1;
    while rest.get(i).is_some_and(|t| t.kind == TokenKind::Dot) {
        let Some(part) = rest.get(i + 1) else {
            return Err(unexpected_end(tokens.get(end), sql));
        };
        if !matches!(
            part.kind,
            TokenKind::Identifier | TokenKind::QuotedIdentifier | TokenKind::Keyword(_)
        ) {
            return Err(Error::syntax_sentence_at(
                format!("syntax error at or near \"{}\"", part.text(sql)),
                pglet_core::codepoint_position(sql, part.start),
            ));
        }
        name.push('.');
        name.push_str(&part.name(sql));
        i += 2;
    }
    expect_reset_end(rest, i, sql)?;
    // Case-insensitive whatever the quoting, as the `SET` reader's name is.
    Ok(Some(name.to_ascii_lowercase()))
}

/// The `42601` for "the statement stopped here", in 18.6's two spellings.
///
/// `terminator` is the `;` token when one was written (`tokens.get(end)`) and `None`
/// when the input simply ran out. The distinction is not cosmetic: the readers slice
/// their argument list as `tokens[1..end]`, which **excludes** the terminator, so an
/// arm that only asked "is there another token in `rest`?" reported `at end of input`
/// for a statement that plainly ended in a `;` — measured against 18.6, `RESET a.;` is
/// `at or near ";"` at `P=9` there and was `at end of input` at `P=10` here.
///
/// The terminator arm carries `P` (a real token offset); the end-of-input arm carries
/// none — see [`Error::syntax_sentence`] for why the length available here is not
/// 18.6's.
fn unexpected_end(terminator: Option<&Token>, sql: &str) -> Error {
    terminator.map_or_else(
        || Error::syntax_sentence("syntax error at end of input"),
        |t| {
            Error::syntax_sentence_at(
                format!("syntax error at or near \"{}\"", t.text(sql)),
                pglet_core::codepoint_position(sql, t.start),
            )
        },
    )
}

/// The word `RESET`'s grammar requires at `rest[at]`.
fn expect_reset_word(
    rest: &[Token],
    at: usize,
    sql: &str,
    word: &str,
    terminator: Option<&Token>,
) -> Result<()> {
    match rest.get(at) {
        Some(t) if is_word(t, sql, word) => Ok(()),
        Some(t) => Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", t.text(sql)),
            pglet_core::codepoint_position(sql, t.start),
        )),
        // `rest` excludes the terminator, so "no more argument tokens" is not the same
        // question as "the input ended" — see `unexpected_end`.
        None => Err(unexpected_end(terminator, sql)),
    }
}

/// `RESET`'s statement end: nothing may follow `rest[at - 1]`.
fn expect_reset_end(rest: &[Token], at: usize, sql: &str) -> Result<()> {
    rest.get(at).map_or(Ok(()), |t| {
        Err(Error::syntax_sentence_at(
            format!("syntax error at or near \"{}\"", t.text(sql)),
            pglet_core::codepoint_position(sql, t.start),
        ))
    })
}

/// Apply a parsed `DISCARD` to a session, answering its command tag.
///
/// What each target reaches in pglet, and what it cannot:
///
/// - **`PLANS`** is a no-op. Postgres's `ResetPlanCache` throws away cached plans
///   and keeps the prepared statements that hold them; pglet re-validates a cached
///   plan against `Schema::generation` on every use, so discarding the cache is
///   unobservable in either engine except as time.
/// - **`SEQUENCES`** clears the session's `currval` cache, which *is* observable:
///   a `currval` after it is `55000` on both engines.
/// - **`TEMP`** is a no-op — pglet has no temporary schema to drop.
/// - **`ALL`** is all of those plus `DEALLOCATE ALL` and `RESET ALL`. The
///   prepared-statement half spares the protocol's unnamed slot for the reason
///   [`PreparedStatements::clear_named_prepared`] states — Postgres's
///   `DropAllPreparedStatements` is the same call `DEALLOCATE ALL` makes.
///
/// **What it does not do:** close the session's open portals. pglet never drops a
/// portal, so a `DISCARD ALL` leaves them where they are; that is the portal
/// lifetime gap, not this statement's, and it is tracked as its own item rather
/// than papered over here.
///
/// # Errors
///
/// `25001` for `DISCARD ALL` inside a transaction block —
/// `PreventInTransactionBlock`, and the only one of the four targets it guards
/// (`PLANS`, `SEQUENCES` and `TEMP` all run inside a block on 18.6, measured).
pub fn execute_discard(
    target: DiscardTarget,
    prepared: &mut impl PreparedStatements,
    sequences: &mut SequenceSession,
    settings: &mut SessionSettings,
    in_transaction_block: bool,
) -> Result<QueryResult> {
    if target == DiscardTarget::All && in_transaction_block {
        return Err(Error::ActiveSqlTransaction { op: "DISCARD ALL" });
    }
    match target {
        DiscardTarget::All => {
            prepared.clear_named_prepared();
            sequences.clear_currval();
            settings.reset_all();
        }
        DiscardTarget::Sequences => sequences.clear_currval(),
        DiscardTarget::Plans | DiscardTarget::Temp => {}
    }
    Ok(QueryResult::ddl(target.tag()))
}

// ============================================================================
// The dispatcher every host drives
// ============================================================================

/// Which of the five session-hygiene statements `sql` is — **the one enumeration**.
///
/// [`is_session_command`] and [`execute_session_command`] were two lists of the same
/// five with nothing coupling them, which is the exact shape that gave `SET`/`SHOW`
/// two answers per host: a sixth member added to one and not the other takes the
/// parser's error at `Parse` instead of its own behaviour. Both now read this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCommand {
    Set,
    Show,
    Deallocate,
    Discard,
    Reset,
}

/// Classify `sql`'s first statement, or `None` when it is not a session command.
///
/// The order is the classifiers' own and does not matter: no two of them can claim
/// one statement, `SET`/`SHOW` being raw-text prefixes and the other three distinct
/// leading words.
fn session_command(sql: &str) -> Option<SessionCommand> {
    if is_set_command(sql) {
        Some(SessionCommand::Set)
    } else if is_show_command(sql) {
        Some(SessionCommand::Show)
    } else if is_deallocate_command(sql) {
        Some(SessionCommand::Deallocate)
    } else if is_discard_command(sql) {
        Some(SessionCommand::Discard)
    } else if is_reset_command(sql) {
        Some(SessionCommand::Reset)
    } else {
        None
    }
}

/// The result shape a `Describe` must answer for a `SHOW`, or `None` when `sql` is
/// not one.
///
/// `SHOW` is the only one of the five that returns a result set, and it is the only
/// one whose `Describe` a host cannot get from a plan: it never reaches the planner,
/// so the plan-side producer of [`ResultShape`] has nothing to answer with and falls
/// back to `NoResultSet`. That fallback made `Describe` of `SHOW server_version`
/// answer `NoData` and `Execute` then send a `DataRow` no field descriptor covers —
/// which a binary client decodes as a row with **no columns**, silently losing the
/// value. The extended protocol is what node-postgres and the fixture harness's
/// binary leg use, so that was every driver that asks a question this seam exists to
/// answer.
///
/// It is derived by asking [`execute_show_command`] rather than by re-deriving the
/// column list, so the shape and the rows cannot disagree. The other four return no
/// result set, which the plan-side fallback already answers correctly. The session
/// settings do not enter it: every modelled parameter is one text column named after
/// itself, whatever the value, which is why this can take a default.
///
/// An unrecognized parameter answers `None` here and takes its `42704` at Execute;
/// 18.6 raises that at Parse instead (parse *analysis*, so `25P02` still outranks it
/// in a failed block). That half is untouched and is `EDGES.md`'s.
#[must_use]
pub fn show_result_shape(sql: &str) -> Option<ResultShape> {
    if !matches!(session_command(sql), Some(SessionCommand::Show)) {
        return None;
    }
    execute_show_command(sql, &SessionSettings::default())
        .ok()
        .map(|r| r.shape)
}

/// Whether `sql` is one of the five session-hygiene statements, and therefore runs
/// through [`execute_session_command`] rather than through the planner.
///
/// A host that routes ahead of the parser reads this; the parser itself rejects
/// every member (`SET`/`SHOW` with `0A000`, the lexer-classified three with
/// `42601`), so the parser's rejection of one of these is never that statement's
/// error. It is [`session_command`], so it cannot fall behind the dispatcher.
#[must_use]
pub fn is_session_command(sql: &str) -> bool {
    session_command(sql).is_some()
}

/// Run one session-hygiene statement, or `None` when `sql` is not one.
///
/// The single home for the routing **and** the answers, so the native server, the
/// embed facade and the wasm bindings over it cannot answer one of these
/// differently. The returned [`QueryResult`] already carries the statement's own
/// command tag — the two `DEALLOCATE` forms and the four `DISCARD` targets report
/// different ones, so a caller must not supply a tag of its own.
///
/// `settings` is the session's own runtime parameters ([`SessionSettings`]); a
/// caller on a wire must compare [`startup_parameters`] across this call and send a
/// `ParameterStatus` for every value that moved, which is Postgres's `GUC_REPORT`
/// rule. `in_transaction_block` is whether *this* session has an explicit block
/// open; it decides `DISCARD ALL` alone.
///
/// # Errors
///
/// Whatever the member statement raises — `42704` for an unknown `SHOW` parameter,
/// `0A000` for a refused `SET` or a `RESET TRANSACTION ISOLATION LEVEL`, `26000` for
/// a `DEALLOCATE` of a name the session lacks, `25001` for `DISCARD ALL` inside a
/// block, `42601` for a syntax error in any of the three lexer-classified ones.
pub fn execute_session_command(
    sql: &str,
    prepared: &mut impl PreparedStatements,
    sequences: &mut SequenceSession,
    settings: &mut SessionSettings,
    in_transaction_block: bool,
) -> Option<Result<QueryResult>> {
    // **Before** the classifier, because this runs for every statement a host hands it
    // and not only for the five it services: this is install site 1 for the ambient session the
    // expression evaluator reads. The enumeration of sites — and why it is stated in
    // exactly one place — is [`SessionSettings::install_ambient`]'s doc.
    settings.install_ambient();
    Some(match session_command(sql)? {
        SessionCommand::Set => execute_set_command(sql, settings, in_transaction_block),
        SessionCommand::Show => execute_show_command(sql, settings),
        SessionCommand::Deallocate => parse_deallocate(sql)
            .and_then(|target| execute_deallocate(&target, prepared))
            .map(QueryResult::ddl),
        SessionCommand::Discard => parse_discard(sql).and_then(|target| {
            execute_discard(target, prepared, sequences, settings, in_transaction_block)
        }),
        SessionCommand::Reset => parse_reset(sql).and_then(|name| {
            match name {
                // `RESET ALL` restores every session setting; a named one restores
                // just that setting, and an unmodelled name is the accepted no-op
                // the GUC family is.
                None => settings.reset_all(),
                // A `RESET` is a `SET … TO DEFAULT`, which fails only for
                // `transaction_isolation` — 18.6's `cannot be reset`.
                Some(name) => {
                    settings.assign(&name, None, SetScope::Session)?;
                }
            }
            Ok(QueryResult::ddl("RESET"))
        }),
    })
}

#[cfg(test)]
mod set_search_path_tests {
    //! The `SET search_path` gate — the one `SET` shape whose value is read.
    //!
    //! No differential fixture can pin the refusal's code: 18.6 *accepts* every one
    //! of these, so the fixture harness's `@expect-error` contract lets any pglet
    //! error pass there. The SQLSTATE and the accept/refuse split are pinned here.

    use super::{
        Error, SessionSettings, check_guc_value_postgres, client_encoding_key, execute_set_command,
        truncate_name_value,
    };

    fn refused(sql: &str) -> String {
        let err = execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
        assert_eq!(err.sqlstate(), "0A000", "{sql}");
        err.to_string()
    }

    fn accepted(sql: &str) {
        let result = execute_set_command(sql, &mut SessionSettings::default(), false)
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        assert_eq!(result.command_tag, "SET");
    }

    #[test]
    fn the_one_schema_is_accepted_in_every_spelling() {
        // Each of these resolves to `public` on 18.6 too — `"$user"` names a schema
        // a fresh database does not have and `pg_catalog` is searched implicitly, so
        // Postgres's own default list is `public` there as well.
        for sql in [
            "SET search_path TO public",
            "SET search_path = public",
            "SET search_path TO PUBLIC",
            r#"SET search_path TO "public""#,
            "SET search_path TO 'public'",
            r#"SET search_path TO "$user", public"#,
            "SET search_path TO '$user', public",
            "SET search_path TO pg_catalog, public",
            "SET search_path TO DEFAULT",
            // A list of elements that select no schema of their own — including the
            // empty one, which is `pg_dump`'s own preamble and the CVE-2018-1058
            // hardening idiom. Accepting it is the decision
            // `check_search_path_elements` documents; the residual it costs (an
            // unqualified name still resolves here, where 18.6 is `42P01`) is
            // `EDGES.md`'s.
            "SET search_path TO pg_catalog",
            "SET search_path TO ''",
            r#"SET search_path TO "$user""#,
            "SET LOCAL search_path TO public",
            "SET SESSION search_path TO public",
            r#"SET "search_path" TO public"#,
            "set search_path to public;",
        ] {
            accepted(sql);
        }
    }

    #[test]
    fn a_schema_pglet_does_not_have_is_refused_loud() {
        // The measured motivating case is the first: a schema-per-test harness
        // (`CREATE SCHEMA test_1; SET search_path TO test_1`) that believed this
        // succeeded read and wrote the one shared schema instead.
        assert_eq!(
            refused("SET search_path TO nosuch"),
            r#"unsupported search_path "nosuch": pglet has only the "public" schema"#
        );
        for sql in [
            "SET search_path TO public, foo",
            "SET search_path TO foo, public",
            "SET search_path = 'nosuch'",
            r#"SET search_path TO "PUBLIC""#,
            "SET LOCAL search_path TO nosuch",
            "SET SESSION search_path TO nosuch",
            // 18.6 splits a string element only on the comma *inside* an unquoted
            // part; `'public, foo'` is one schema literally named `public, foo`.
            "SET search_path = 'public, foo'",
            // A value this reader cannot parse is refused rather than accepted —
            // a value it cannot see is one it must not silently honour.
            "SET search_path",
            "SET search_path TO",
            "SET search_path TO public.foo",
        ] {
            refused(sql);
        }
    }

    #[test]
    fn the_function_spelling_reaches_the_same_decider() {
        // The gate this replaced hung off `is_set_command`, so
        // `SELECT pg_catalog.set_config('search_path', 'nosuch', false)` — the same
        // request, and the line modern `pg_dump` emits — walked straight past it and
        // the session went on reading and writing `public`. Measured on 18.6:
        // `set_config('search_path','nosuch',false)` then `SELECT * FROM t` is
        // `42P01` there and answered the row here.
        //
        // These call the decider directly; `planner::expr`'s `set_config` arm is what
        // routes to it, and `commands/set_config_search_path_*` are the differential
        // fixtures over the whole statement.
        for value in ["nosuch", "public, foo", "PUBLIC, nosuch", "a, b"] {
            let err = super::check_search_path_value(value).expect_err(value);
            assert_eq!(err.sqlstate(), "0A000", "{value}");
        }
        for value in [
            "public",
            "PUBLIC",
            "\"public\"",
            "\"$user\", public",
            "$user, public",
            "pg_catalog, public",
            "  public  ",
            // `pg_dump`'s own preamble, and the empty list's other spellings.
            "",
            "\"\"",
            "pg_catalog",
        ] {
            super::check_search_path_value(value).unwrap_or_else(|e| panic!("{value:?}: {e}"));
        }
        // The two routes read a string element differently, and must: 18.6's do too.
        // `SET search_path = 'a, b'` is one schema literally named `a, b`
        // (`flatten_set_variable_args` quotes each string element before the list is
        // stored), while `set_config('search_path','a, b',false)` stores the string
        // raw and it is two. Both are refused here, by the one decider.
        refused("SET search_path = 'a, b'");
        assert_eq!(
            super::split_guc_list("a, b"),
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(super::split_guc_list("\"a, b\""), vec!["a, b".to_string()]);
        assert!(super::split_guc_list("").is_empty());
    }

    #[test]
    fn both_routes_agree_on_a_long_valid_list() {
        // The reader used to stop at a token cap, which broke the binding rule twice
        // over: first it *accepted* `SET search_path TO <31 × public>, evil_schema` on
        // the prefix it could see, and then — capped-and-refusing — it refused a
        // 40-element all-`public` list that 18.6 accepts and that the `set_config`
        // route (which reads a whole string) took. Same request, two answers. The
        // statement reader has no cap now, so this runs the *same* list through both.
        for copies in [1, 29, 30, 31, 40, 100] {
            let list = std::iter::repeat_n("public", copies)
                .collect::<Vec<_>>()
                .join(", ");
            accepted(&format!("SET search_path TO {list}"));
            super::check_search_path_value(&list)
                .unwrap_or_else(|e| panic!("{copies} × public via set_config: {e}"));
            refused(&format!("SET search_path TO {list}, evil_schema"));
            let err = super::check_search_path_value(&format!("{list}, evil_schema"))
                .expect_err("evil tail");
            assert_eq!(err.sqlstate(), "0A000", "{copies}");
        }
    }

    #[test]
    fn a_zero_length_delimited_identifier_is_the_scanners_error_not_this_gate() {
        // The regression this closed: the decider's "an empty element is the empty
        // list" arm was written for a zero-length *string* (`''`, which 18.6 accepts
        // and which is `pg_dump`'s preamble) and could not tell it from a zero-length
        // *delimited identifier* (`""`), because both arrive as an empty `Token::name`.
        // On 18.6 they are not the same thing: `''` is an empty GUC list and `""` is a
        // scanner error. The refusal is the lexer's now, so this reader never sees the
        // token — which is why the code is `42601` and not this gate's `0A000`.
        for sql in [
            r#"SET search_path TO """#,
            r#"SET search_path TO public, """#,
            r#"SET LOCAL search_path TO """#,
        ] {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            assert_eq!(err.sqlstate(), "42601", "{sql}");
        }
        // The string spellings the arm was written for stay accepted.
        for sql in ["SET search_path TO ''", r#"SET search_path TO '""'"#] {
            accepted(sql);
        }
    }

    /// A `client_encoding` longer than 63 bytes is cut where 18.6's `truncate_identifier` cuts
    /// it — at a character boundary, never inside one. Byte 63 of `ab` + 31 `é` + `utf8` falls
    /// inside the 31st `é`, so the name kept is `ab` + 30 `é` (62 bytes), and 18.6 answers
    /// `22023 invalid value for parameter "client_encoding": "ab…"` with exactly that name
    /// (measured). A cut at byte 63 itself would split the character and panic.
    #[test]
    fn a_long_encoding_name_is_cut_at_a_character_boundary() {
        let straddling = format!("ab{}utf8", "é".repeat(31));
        let kept = format!("ab{}", "é".repeat(30));
        assert_eq!(truncate_name_value(&straddling), kept);
        assert_eq!(client_encoding_key(&straddling), "ab");
        let Err(Error::InvalidParameterValue { message, .. }) =
            check_guc_value_postgres("client_encoding", Some(&straddling))
        else {
            panic!("expected 22023");
        };
        assert_eq!(
            message,
            format!("invalid value for parameter \"client_encoding\": \"{kept}\"")
        );
        // on a boundary: the 63 bytes are kept whole
        let on_boundary = format!("a{}utf8", "é".repeat(31));
        assert_eq!(truncate_name_value(&on_boundary).len(), 63);
        // and the `SET` door answers it without a panic
        let sql = format!("SET client_encoding = '{straddling}'");
        let err = execute_set_command(&sql, &mut SessionSettings::default(), false)
            .expect_err("a name 18.6 does not know");
        assert_eq!(err.sqlstate(), "22023");
    }

    /// `SET NAMES` reads `gram.y`'s `NAMES opt_encoding` — a string, `DEFAULT` or nothing —
    /// and each refusal carries 18.6's own message, compared here because a fixture compares only
    /// the SQLSTATE. Every message below is 18.6's for the same statement (measured).
    #[test]
    fn set_names_answers_as_the_grammar_does() {
        let refusal = |sql: &str| {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            (err.sqlstate(), err.to_string())
        };
        let near = |token: &str| ("42601", format!("syntax error at or near \"{token}\""));
        assert_eq!(refusal("SET NAMES 'utf8' 'x'"), near("'x'"));
        assert_eq!(refusal("SET NAMES 'utf8', 'latin1'"), near(","));
        assert_eq!(refusal("SET NAMES DEFAULT garbage"), near("garbage"));
        assert_eq!(refusal("SET NAMES 'utf8' /* c */ 'x'"), near("'x'"));
        assert_eq!(refusal("SET NAMES utf8"), near("utf8"));
        assert_eq!(refusal("SET NAMES \"utf8\""), near("\"utf8\""));
        assert_eq!(refusal("SET NAMES 1"), near("1"));
        assert_eq!(refusal("SET LOCAL NAMES utf8"), near("utf8"));
        let unknown = (
            "42704",
            "unrecognized configuration parameter \"names\"".to_string(),
        );
        assert_eq!(refusal("SET NAMES TO 'utf8'"), unknown);
        assert_eq!(refusal("SET NAMES = 'utf8'"), unknown);
        assert_eq!(refusal("SET NAMES TO DEFAULT"), unknown);
        assert_eq!(
            refusal("SET NAMES TO"),
            ("42601", "syntax error at end of input".to_string())
        );
        // with `TO`/`=`, 18.6 parses the list before it looks the setting up
        assert_eq!(refusal("SET NAMES TO 'a' 'b'"), near("'b'"));
        assert_eq!(refusal("SET NAMES TO NULL"), near("NULL"));
        assert_eq!(refusal("SET NAMES = ,"), near(","));
        assert_eq!(refusal("SET NAMES TO DEFAULT, 'x'"), near(","));
        assert_eq!(refusal("SET NAMES TO - x"), near("x"));
        assert_eq!(
            refusal("SET NAMES TO 'a',"),
            ("42601", "syntax error at end of input".to_string())
        );
        let one = ("22023", "SET names takes only one argument".to_string());
        assert_eq!(refusal("SET NAMES TO 'a', 'b'"), one);
        assert_eq!(refusal("SET NAMES = 1.5, -2"), one);
        for sql in [
            "SET NAMES TO utf8",
            "SET NAMES TO \"utf8\"",
            "SET NAMES TO -1",
            "SET NAMES TO LOCAL",
        ] {
            assert_eq!(refusal(sql), unknown, "{sql}");
        }
        assert_eq!(refusal("SET NAMES FROM CURRENT"), unknown);
        assert_eq!(refusal("SET NAMES FROM x"), near("x"));
        assert_eq!(refusal("SET NAMES FROM CURRENT x"), near("x"));
        for sql in [
            "SET NAMES 'utf8'",
            "SET NAMES DEFAULT",
            "SET NAMES",
            "SET NAMES E'utf8'",
        ] {
            accepted(sql);
        }
    }

    #[test]
    fn client_encoding_is_refused_unless_it_is_utf8() {
        // pglet is UTF-8 end to end and its startup packet asserts `client_encoding=
        // UTF8`; accepting a `SET` to anything else and then sending UTF-8 bytes is the
        // one modelled-parameter divergence that corrupts data. 18.6 accepts and
        // converts, so the refusal is pglet-stricter. All three of Postgres's UTF-8
        // spellings pass (measured on 18.6), and `SET NAMES` is `gram.y`'s other
        // spelling of the same GUC — a gate with a spelling that walks past it is the
        // defect the fold closes.
        for sql in [
            "SET client_encoding TO 'UTF8'",
            "SET client_encoding TO 'UTF-8'",
            "SET client_encoding TO 'UNICODE'",
            "SET client_encoding TO utf8",
            "SET client_encoding TO DEFAULT",
            "SET NAMES 'UTF8'",
            "SET NAMES 'utf-8'",
            // `SQL_ASCII` is not a UTF-8 spelling — it is "no conversion", which is
            // exactly what pglet does, so refusing it would buy nothing and would break
            // the `pg_dump` preamble of a `SQL_ASCII` database. Postgres's encoding-name
            // match ignores case, `-` and `_`, and 18.6 accepts all four of these.
            "SET client_encoding TO 'SQL_ASCII'",
            "SET client_encoding TO 'sql_ascii'",
            "SET client_encoding TO 'sqlascii'",
            "SET client_encoding = 'SQL-ASCII'",
            "SET NAMES 'SQL_ASCII'",
        ] {
            accepted(sql);
        }
        super::check_guc_value("client_encoding", Some("SQL_ASCII")).expect("sql_ascii");
        for sql in [
            "SET client_encoding TO 'LATIN1'",
            "SET client_encoding = 'EUC_JP'",
            "SET LOCAL client_encoding TO 'WIN1252'",
            "SET NAMES 'LATIN1'",
        ] {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            assert_eq!(err.sqlstate(), "0A000", "{sql}");
            assert!(err.to_string().contains("UTF8-only"), "{sql}: {err}");
        }
        // The same decider, reached from the function route.
        super::check_guc_value("client_encoding", Some("UTF8")).expect("utf8");
        super::check_guc_value("client_encoding", None).expect("a reset");
        assert_eq!(
            super::check_guc_value("client_encoding", Some("LATIN1"))
                .expect_err("latin1")
                .sqlstate(),
            "0A000"
        );
    }

    #[test]
    fn a_fixed_parameter_takes_only_the_value_pglet_has() {
        // pglet has one session zone, one date style, one interval style and one reading of
        // a string literal. 18.6 acts on a `SET` that moves one; accepting it here left the
        // session answering as though it had not been sent, so it is refused — and a
        // spelling of the value pglet already has is honored, on every route.
        for sql in [
            "SET TIME ZONE 'UTC'",
            "SET TIME ZONE utc",
            "SET TIME ZONE \"UTC\"",
            "SET time zone 'uTc';",
            "SET SESSION TIME ZONE 'UTC'",
            "SET LOCAL TIME ZONE 'UTC'",
            // `zone_value`'s resets
            "SET TIME ZONE DEFAULT",
            "SET TIME ZONE LOCAL",
            "SET LOCAL TIME ZONE LOCAL",
            "SET timezone = 'UTC'",
            "SET \"TimeZone\" TO utc",
            "SET timezone TO DEFAULT",
            "SET DateStyle = 'ISO, MDY'",
            // a list setting: two words are one value
            "SET DateStyle = ISO, MDY",
            "SET datestyle TO iso,mdy",
            "SET datestyle = ISO",
            "SET datestyle = 'mdy'",
            "SET datestyle = ''",
            "SET datestyle = DEFAULT",
            "SET IntervalStyle = postgres",
            "SET intervalstyle TO 'POSTGRES'",
            "SET standard_conforming_strings = on",
            "SET standard_conforming_strings = 'ON'",
            "SET standard_conforming_strings TO true",
            "SET standard_conforming_strings = 1",
            "SET standard_conforming_strings = DEFAULT",
        ] {
            accepted(sql);
        }
        for (sql, fixed) in [
            ("SET TIME ZONE 'America/New_York'", "UTC"),
            ("SET SESSION TIME ZONE 'America/New_York'", "UTC"),
            ("SET LOCAL TIME ZONE 'America/New_York'", "UTC"),
            ("SET timezone = 'America/New_York'", "UTC"),
            ("SET \"TimeZone\" TO 'America/New_York'", "UTC"),
            // a zero offset under another name is shown under that name on 18.6
            ("SET TIME ZONE 'GMT'", "UTC"),
            ("SET TIME ZONE 'Etc/UTC'", "UTC"),
            ("SET TIME ZONE 0", "UTC"),
            ("SET TIME ZONE -5", "UTC"),
            ("SET timezone = '+00'", "UTC"),
            // `LOCAL` is a reset only as `zone_value`; a zone named `local` is not
            ("SET timezone = local", "UTC"),
            ("SET timezone = 'local'", "UTC"),
            ("SET TIME ZONE 'local'", "UTC"),
            // surrounding whitespace is part of the value (18.6: `22023`)
            ("SET TIME ZONE ' UTC'", "UTC"),
            ("SET intervalstyle = ' postgres'", "postgres"),
            ("SET DateStyle = 'German'", "ISO, MDY"),
            ("SET DateStyle = ISO, DMY", "ISO, MDY"),
            ("SET datestyle TO sql", "ISO, MDY"),
            ("SET datestyle = 'ISO, MDY, DMY'", "ISO, MDY"),
            ("SET IntervalStyle = 'iso_8601'", "postgres"),
            ("SET intervalstyle TO sql_standard", "postgres"),
            ("SET standard_conforming_strings = off", "on"),
            ("SET standard_conforming_strings TO 'of'", "on"),
            ("SET standard_conforming_strings = 0", "on"),
        ] {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            assert_eq!(err.sqlstate(), "0A000", "{sql}");
            assert!(
                err.to_string()
                    .ends_with(&format!("pglet's is always \"{fixed}\"")),
                "{sql}: {err}"
            );
        }
        // The message names the parameter and the value as written.
        assert_eq!(
            refused("SET TIME ZONE 'America/New_York'"),
            "not implemented: timezone \"America/New_York\" is not supported: pglet's is always \"UTC\""
        );
        // A shape the one-value reader cannot read stays loud: an offset's `INTERVAL`
        // spelling and `FROM CURRENT` (18.6 takes both), a missing value and a trailing
        // token (`42601` there too).
        for sql in [
            "SET TIME ZONE INTERVAL '0' HOUR TO MINUTE",
            "SET TIME ZONE",
            "SET TIME ZONE;",
            "SET TIME ZONE 'UTC' 'UTC'",
            "SET TIME ZONE LOCAL 'UTC'",
            "SET timezone FROM CURRENT",
        ] {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            assert_eq!(err.sqlstate(), "42601", "{sql}");
        }
        // 18.6's own verdicts: a value that is no Boolean, a list for a scalar setting.
        for (sql, message) in [
            (
                "SET standard_conforming_strings = 'maybe'",
                "parameter \"standard_conforming_strings\" requires a Boolean value",
            ),
            (
                "SET timezone = 'UTC', 'UTC'",
                "SET timezone takes only one argument",
            ),
            (
                "SET intervalstyle = postgres, postgres",
                "SET intervalstyle takes only one argument",
            ),
        ] {
            let err =
                execute_set_command(sql, &mut SessionSettings::default(), false).expect_err(sql);
            assert_eq!(
                (err.sqlstate(), err.to_string().as_str()),
                ("22023", message),
                "{sql}"
            );
        }
        // The same decider, reached from the function route and the startup packet; `None`
        // is a reset.
        for name in [
            "timezone",
            "datestyle",
            "intervalstyle",
            "standard_conforming_strings",
        ] {
            let fixed = super::fixed_guc_value(name).expect(name);
            assert!(super::guc_value_is_checked(name), "{name}");
            super::check_guc_value(name, Some(fixed)).expect(name);
            super::check_guc_value(name, None).expect(name);
            assert_eq!(
                super::check_guc_value(name, Some("nosuch"))
                    .expect_err(name)
                    .sqlstate(),
                if name == "standard_conforming_strings" {
                    "22023"
                } else {
                    "0A000"
                },
                "{name}"
            );
            // …and it is the value `SHOW` answers, so an accepted `SET` cannot have moved it.
            let shown =
                super::execute_show_command(&format!("SHOW {name}"), &SessionSettings::default())
                    .expect(name);
            assert_eq!(
                shown.rows,
                vec![vec![crate::types::Value::Text(fixed.into())]],
                "{name}"
            );
        }
        assert_eq!(super::fixed_guc_value("application_name"), None);
    }

    #[test]
    fn every_other_guc_stays_an_accepted_no_op() {
        // The GUC-echo family, which is what lets a driver's connection-setup batch
        // through. `SET SESSION CHARACTERISTICS …` is in here because its second
        // word is not a GUC name and must not be read as one.
        for sql in [
            "SET client_encoding TO 'UTF8'",
            "SET application_name = 'app'",
            "SET TIME ZONE 'UTC'",
            "SET statement_timeout = 0",
            "SET extra_float_digits = 3",
            "SET nosuchguc = 1",
            "SET search_paths TO nosuch",
            "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED",
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            "SET TRANSACTION READ WRITE",
        ] {
            accepted(sql);
        }
    }

    #[test]
    fn a_transaction_mode_set_refuses_read_only() {
        // The `BEGIN`-side refusal's other half: accepting the request and then
        // executing the write it forbids is the silent over-acceptance, whichever
        // statement asked.
        for sql in [
            "SET TRANSACTION READ ONLY",
            "SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY",
            "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE, READ ONLY, DEFERRABLE",
        ] {
            assert_eq!(
                refused(sql),
                "not implemented: READ ONLY transactions are not supported"
            );
        }
    }
}

#[cfg(test)]
mod discard_reset_tests {
    //! `DISCARD` / `RESET` shape, and what `DISCARD` reaches.

    use std::collections::HashMap;

    use super::{
        DiscardTarget, SessionSettings, execute_discard, execute_session_command,
        is_discard_command, is_reset_command, parse_discard, parse_reset,
    };
    use crate::executor::SequenceSession;

    fn target(sql: &str) -> DiscardTarget {
        parse_discard(sql).unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The message `Error::Syntax` carries, without the `syntax error: ` prefix its
    /// `Display` prepends. The prefix is pglet's own and doubles the words on the
    /// wire (`syntax error: syntax error at or near "FOO"` where 18.6 sends
    /// `syntax error at or near "FOO"`); it is `Error::Syntax`'s to fix for every
    /// caller at once, not this statement's, and `EDGES.md` carries the pair.
    fn syntax_error(sql: &str) -> String {
        let err = parse_discard(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "42601", "{sql}");
        let crate::Error::Syntax { message, .. } = err else {
            panic!("{sql}: not a syntax error");
        };
        message
    }

    #[test]
    fn the_five_words_map_to_four_targets() {
        // 18.6 reports command tag `DISCARD TEMP` for `DISCARD TEMPORARY`, measured
        // — the two spellings are one target, not two.
        assert_eq!(target("DISCARD ALL"), DiscardTarget::All);
        assert_eq!(target("DISCARD PLANS"), DiscardTarget::Plans);
        assert_eq!(target("DISCARD SEQUENCES"), DiscardTarget::Sequences);
        assert_eq!(target("DISCARD TEMP"), DiscardTarget::Temp);
        assert_eq!(target("DISCARD TEMPORARY"), DiscardTarget::Temp);
        assert_eq!(target("DISCARD TEMPORARY").tag(), "DISCARD TEMP");
    }

    #[test]
    fn the_classifier_reads_the_lexer_not_the_raw_text() {
        // A pool's check-in statement arrives from a query-tagging client with a
        // comment on the front, in whatever case the driver wrote it, and with the
        // terminator the extended protocol's `Parse` does not strip. All three are
        // the same statement.
        for sql in [
            "discard all",
            "DISCARD  ALL",
            "/* trace */ DISCARD ALL",
            "-- trace\nDISCARD ALL",
            "DISCARD ALL;",
            "  DISCARD\tALL  ",
        ] {
            assert!(is_discard_command(sql), "{sql}");
            assert_eq!(target(sql), DiscardTarget::All, "{sql}");
        }
        // And the word stays usable as a name, as it is on 18.6 where `DISCARD` is
        // UNRESERVED.
        assert!(!is_discard_command("SELECT discard FROM t"));
        assert!(!is_reset_command("SELECT reset FROM t"));
        assert!(!is_discard_command(r#""discard" all"#));
    }

    #[test]
    fn a_malformed_discard_is_the_syntax_error_18_6_raises() {
        // The unknown-word message carries the word **as written**: 18.6 raises it
        // from the scanner, ahead of any fold.
        assert_eq!(
            syntax_error("DISCARD FOO"),
            r#"syntax error at or near "FOO""#
        );
        assert_eq!(
            syntax_error("DISCARD ALL x"),
            r#"syntax error at or near "x""#
        );
        assert_eq!(syntax_error("DISCARD"), "syntax error at end of input");
        assert_eq!(syntax_error("DISCARD;"), r#"syntax error at or near ";""#);
    }

    #[test]
    fn reset_takes_a_name_and_does_nothing_with_it() {
        // Every GUC pglet models is a fixed value it never moves, so restoring one
        // is a no-op; the shape check is not. 18.6 reports tag `RESET` for `RESET
        // ALL` too, measured — not `RESET ALL`.
        for sql in [
            "RESET ALL",
            "reset all",
            "RESET search_path",
            "RESET nosuchguc",
        ] {
            assert!(is_reset_command(sql), "{sql}");
            parse_reset(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
        for (sql, want) in [
            ("RESET", "syntax error at end of input"),
            ("RESET ALL x", r#"syntax error at or near "x""#),
            ("RESET ALL, search_path", r#"syntax error at or near ",""#),
        ] {
            let err = parse_reset(sql).expect_err(sql);
            assert_eq!(err.sqlstate(), "42601", "{sql}");
            let crate::Error::Syntax { message, .. } = err else {
                panic!("{sql}: not a syntax error");
            };
            assert_eq!(message, want, "{sql}");
        }
    }

    #[test]
    fn reset_takes_gram_ys_reset_rest_not_one_word() {
        // `reset_rest` is `generic_reset` **plus** three fixed spellings that name a
        // GUC the words do not spell. Reading one word made each a `42601` here where
        // 18.6 answers `RESET` — and `RESET SESSION AUTHORIZATION` is a real pool
        // check-in statement. Measured pairs, 18.6 first:
        //   RESET TIME ZONE               RESET            was 42601 at "ZONE"
        //   RESET SESSION AUTHORIZATION   RESET            was 42601 at "AUTHORIZATION"
        //   RESET foo.bar                 RESET            was 42601 at "."
        //   RESET TRANSACTION ISOLATION LEVEL
        //                                 0A000 parameter "transaction_isolation"
        //                                 cannot be reset  was 42601 at "ISOLATION"
        assert_eq!(
            parse_reset("RESET TIME ZONE").expect("TIME ZONE"),
            Some("timezone".to_string())
        );
        assert_eq!(
            parse_reset("RESET SESSION AUTHORIZATION").expect("SESSION AUTHORIZATION"),
            Some("session_authorization".to_string())
        );
        assert_eq!(
            parse_reset("reset Foo.Bar").expect("dotted var_name"),
            Some("foo.bar".to_string())
        );
        let err = parse_reset("RESET TRANSACTION ISOLATION LEVEL").expect_err("refused");
        assert_eq!(err.sqlstate(), "0A000");
        // The one-word spellings of the same leading words stay `generic_reset`:
        // `RESET TIME` is the GUC named `time` (18.6: `42704`; here the GUC family's
        // accepted no-op), not a truncated `TIME ZONE`.
        assert_eq!(
            parse_reset("RESET TIME").expect("generic"),
            Some("time".to_string())
        );
        assert_eq!(
            parse_reset("RESET SESSION").expect("generic"),
            Some("session".to_string())
        );
        // And a half-written fixed spelling is still a syntax error.
        for sql in [
            "RESET TIME NOSUCH",
            "RESET SESSION NOSUCH",
            "RESET TRANSACTION ISOLATION",
            "RESET foo.",
        ] {
            let err = parse_reset(sql).expect_err(sql);
            assert_eq!(err.sqlstate(), "42601", "{sql}");
        }
    }

    #[test]
    fn the_routing_predicate_cannot_fall_behind_the_dispatcher() {
        // They were two lists of the same five with nothing coupling them, which is
        // the exact shape that gave `SET`/`SHOW` two answers per host: a member the
        // dispatcher claims and the predicate does not takes the parser's error at
        // `Parse` instead of its own behaviour. Both read `session_command` now, and
        // this is the assertion that says so over a corpus rather than by reading it.
        for sql in [
            "SET application_name = 'x'",
            "SHOW server_version",
            "DEALLOCATE ALL",
            "DISCARD ALL",
            "RESET ALL",
            "/* c */ DISCARD ALL",
            "reset time zone",
            "SELECT 1",
            "BEGIN",
            "SETTING",
            "SELECT discard FROM t",
            "",
        ] {
            let claimed = execute_session_command(
                sql,
                &mut (),
                &mut SequenceSession::new(),
                &mut SessionSettings::default(),
                false,
            )
            .is_some();
            assert_eq!(
                super::is_session_command(sql),
                claimed,
                "{sql:?}: the predicate and the dispatcher disagree"
            );
        }
    }

    #[test]
    fn a_session_setting_moves_and_is_restored() {
        // `application_name` is the one runtime parameter whose value is the
        // session's rather than the build's: 18.6 carries it in the startup packet,
        // reports it as `GUC_REPORT`, and lets `SET` move it. A fixed answer here
        // would be a claim about the client.
        let mut settings = SessionSettings::new("driver".into());
        let run = |sql: &str, settings: &mut SessionSettings| {
            execute_session_command(sql, &mut (), &mut SequenceSession::new(), settings, false)
                .expect("claimed")
                .unwrap_or_else(|e| panic!("{sql}: {e}"))
        };
        assert_eq!(settings.application_name(), "driver");
        run("SET application_name = 'probe'", &mut settings);
        assert_eq!(settings.application_name(), "probe");
        // `RESET` restores the **startup packet's** value, not a compile-time
        // default — measured on 18.6, whose `DISCARD ALL` reports the packet's value
        // back to a connection that started with one.
        run("RESET application_name", &mut settings);
        assert_eq!(settings.application_name(), "driver");
        run("SET application_name TO 'again'", &mut settings);
        run("RESET ALL", &mut settings);
        assert_eq!(settings.application_name(), "driver");
        run("SET application_name TO 'again'", &mut settings);
        run("DISCARD ALL", &mut settings);
        assert_eq!(settings.application_name(), "driver");
        // `SET application_name TO DEFAULT` is the third reset spelling.
        run("SET application_name TO 'again'", &mut settings);
        run("SET application_name TO DEFAULT", &mut settings);
        assert_eq!(settings.application_name(), "driver");
        // And `SHOW` answers it — 18.6 does, and this used to be `42704`.
        let shown = run("SHOW application_name", &mut settings);
        assert_eq!(
            shown.rows,
            vec![vec![crate::types::Value::Text("driver".into())]]
        );
    }

    #[test]
    fn discard_all_clears_the_named_prepared_statements_and_currval() {
        // `DISCARD ALL` is `DEALLOCATE ALL` + `DISCARD PLANS/SEQUENCES/TEMP` +
        // `RESET ALL`, so the prepared-statement half spares the protocol's unnamed
        // slot exactly as `DEALLOCATE ALL` does (Postgres calls the same
        // `DropAllPreparedStatements`).
        let mut prepared: HashMap<String, u8> = HashMap::new();
        prepared.insert(String::new(), 0);
        prepared.insert("s1".into(), 1);
        let mut sequences = SequenceSession::new();
        let mut settings = SessionSettings::default();
        let result = execute_discard(
            DiscardTarget::All,
            &mut prepared,
            &mut sequences,
            &mut settings,
            false,
        )
        .expect("outside a block");
        assert_eq!(result.command_tag, "DISCARD ALL");
        assert_eq!(prepared.keys().collect::<Vec<_>>(), vec![""]);
    }

    #[test]
    fn only_discard_all_is_refused_inside_a_transaction_block() {
        // `PreventInTransactionBlock` guards `DISCARD ALL` alone: the other three
        // targets run inside a block on 18.6, measured.
        let err = execute_discard(
            DiscardTarget::All,
            &mut (),
            &mut SequenceSession::new(),
            &mut SessionSettings::default(),
            true,
        )
        .expect_err("in a block");
        assert_eq!(err.sqlstate(), "25001");
        assert_eq!(
            err.to_string(),
            "DISCARD ALL cannot run inside a transaction block"
        );
        for target in [
            DiscardTarget::Plans,
            DiscardTarget::Sequences,
            DiscardTarget::Temp,
        ] {
            execute_discard(
                target,
                &mut (),
                &mut SequenceSession::new(),
                &mut SessionSettings::default(),
                true,
            )
            .unwrap_or_else(|e| panic!("{target:?} in a block: {e}"));
        }
    }

    #[test]
    fn the_dispatcher_claims_all_five_and_nothing_else() {
        // The routing is one predicate on every host; a member this misses answers
        // the parser's rejection instead of its own behaviour, which is exactly how
        // `SET`/`SHOW` came to have two answers depending on the host.
        for sql in [
            "SET application_name = 'x'",
            "SHOW server_version",
            "DEALLOCATE ALL",
            "DISCARD ALL",
            "RESET ALL",
        ] {
            let outcome = execute_session_command(
                sql,
                &mut (),
                &mut SequenceSession::new(),
                &mut SessionSettings::default(),
                false,
            );
            assert!(outcome.is_some(), "{sql}");
            outcome
                .expect("claimed")
                .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
        for sql in ["SELECT 1", "BEGIN", "INSERT INTO t VALUES (1)", "SETTING"] {
            assert!(
                execute_session_command(
                    sql,
                    &mut (),
                    &mut SequenceSession::new(),
                    &mut SessionSettings::default(),
                    false
                )
                .is_none(),
                "{sql}"
            );
        }
    }
}
