//! The statement-depth bounds: how deep a statement's walkers may recurse before the
//! statement is refused `54001 stack depth limit exceeded`.
//!
//! Almost every pass over a statement — the fused planner's passes inside the parser,
//! the executor's evaluation, the deparser, `Drop`/`Clone`/`serde` on the IR — recurses
//! once per tree level. Two bounds keep that recursion on the stack, each covering the
//! walks the other cannot:
//!
//! - **The byte check** ([`check_stack`]), native only: 18.6's `check_stack_depth`. Every
//!   walker that can fail takes it once a level, and refuses when its frame is within
//!   this thread's reserve of the stack's low end — declared by the host that sized the
//!   thread, or a Linux main thread's `[stack]` — or past a budget below the host's entry
//!   where the low end is not known (the `stack` module states the rules, and why no
//!   other thread's stack is learned). An infallible walk too dear a level for the
//!   reserve checks for the whole walk's room before it starts ([`check_stack_room`]:
//!   a plan's `Clone`, [`clone_plan`]).
//! - **The count** ([`check`], [`max_depth`]): where a tree is **built**, the parser
//!   measures each finished expression and each plan chain it extends, and no tree
//!   deeper than this thread's limit reaches a walker. It is what bounds the walks that
//!   cannot fail — `Clone`, `Drop`, `PartialEq`, the type walks, the `contains_*`
//!   visitors — and the reserve is sized to hold one of them over a whole tree at the
//!   limit ([`stack_reserve`]). On wasm, whose host stack a module cannot observe, it is
//!   the whole bound.
//!
//! A plan chain past the count, and nesting past it, are refused on the spot. An
//! operator chain past it is replaced by a one-level `NULL` placeholder and the
//! statement parses on, so that an error 18.6 raises *before* the analysis where it
//! would raise its own `54001` can still outrank the refusal (`Parser::refusal_answer`).
//! A byte-check refusal is raised on the spot, where the walker runs, and takes the
//! same precedence as a set-operation chain's (`EDGES.md` row 171). The measurement
//! itself ([`expr_depth`], [`plan_depth`]) keeps its own explicit stack.
//!
//! A stored expression is read back through the catalog encoding, whose walks take
//! both bounds a level ([`EncodingLevel`]): a store written where the bounds were larger
//! is refused `54001` by a smaller opener, never overflowing it, and a DDL statement
//! refuses an expression its own thread could not encode and decode again
//! ([`check_stored_expr`]).
//!
//! # The catalog is walked by statements that did not write it
//!
//! A statement walks the **whole** catalog: a transaction block's snapshot is a `Clone`
//! of it, a rollback drops one, and a commit encodes it where the store keeps it. The
//! `Clone` and the `Drop` cannot fail, and the encoding runs after the statement's own
//! work — so a catalog deeper than the calling thread walks is not a statement's own
//! error to raise: it aborted the process, or answered `54001` for a write already
//! made. Every host therefore asks [`check_catalog_walks`] at each statement's
//! **entry**, before anything of the statement runs, against the catalog's own depth
//! ([`catalog_depth`], held without a walk per statement by [`CatalogDepth`]). The same
//! question is asked where an expression is stored ([`check_stored_expr`]), from deeper
//! in the stack than any entry, so a thread that accepts an expression can still walk
//! its catalog afterwards.
//!
//! # The limits are per thread
//!
//! The count and the byte check's reserve are thread-locals, set by the host that owns
//! the thread: the native server declares its connection threads' stack and raises the
//! count on them ([`SERVER_THREAD_STACK`] / [`SERVER_MAX_DEPTH`]), and every other
//! thread starts at [`DEFAULT_MAX_DEPTH`] — [`FALLBACK_MAX_DEPTH`] where its stack is not
//! known — sized for the smallest stack a caller can be assumed to have. The embed facade
//! exposes both setters (`pglet::set_thread_max_depth`, `pglet::set_thread_stack`).
//! `AND`/`OR` lists are not deep (a long one is built balanced — `Expr::bool_list`). The
//! parser's own nesting counts against the limit too.
//!
//! **Each constant is a measurement, and the instrument is
//! `crates/pglet/tests/stack_depth.rs`**: it drives every deep-tree shape to its edge on
//! threads of known stack through the whole pipeline, and past it, and aborts if a bound
//! is too loose. `crates/pglet_server/tests/stack_depth.rs` drives the server's pair,
//! and `tests-web/integration/depth_edge.ts` the wasm limit under both of V8's tiers,
//! over every arm of the parser's operator loop — the kinds the arms build, which
//! `parser::operator_loop_arms` reads from the builders and holds both instruments to.
//! The count is one half of a budget with `parser::MAX_DEPTH` (nesting is paid in parser
//! frames) and, through the frame size, with `pglet_core`'s `MAX_ERROR_SIZE`.

use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::catalog::{
    CheckConstraint, Column, ColumnDefault, ConstraintKind, IdentityGeneration, Index, IndexKey,
    Schema, Table, Uniqueness,
};
use crate::planner::{AddConstraint, Expr, InsertRows, OnConflictAction, Plan};
use crate::{Error, Result};

/// The limit a thread starts with, sized for a 2 MiB stack.
///
/// 2 MiB is Rust's spawned-thread and libtest default, and the stack a thread whose
/// stack the engine knows — declared, or a Linux main thread's — is assumed to have at
/// least; a thread whose stack it does not know starts at [`FALLBACK_MAX_DEPTH`], sized
/// for the smallest stack a thread has by default on the target (a 1 MiB Windows main
/// thread). The count and the byte check share it: the
/// reserve holds an infallible walk over this many levels ([`stack_reserve`]: 256 KiB +
/// 2 600 × 224 B = 824.75 KiB), and what remains holds the fallible walkers to about this
/// depth on the cheapest chain (401 B a level for a `+` chain, measured). The instrument
/// answers a `+` chain through 2 597 terms on a declared 2 MiB thread, and a `CHECK`
/// through 1 531 — its encoding's walks are the dearer; on an undeclared one, measured
/// against the budget below the facade entry, 2 316 and 1 358.
#[cfg(all(not(target_arch = "wasm32"), not(debug_assertions)))]
pub const DEFAULT_MAX_DEPTH: usize = 2600;
/// The release constant's 2 MiB budget over **debug** frames.
///
/// The reserve is 256 KiB + 256 × 1 792 B = 704 KiB. A `+` chain costs about 3.8 KiB a level there,
/// and a level of parser nesting far more — 19 KiB a nested call, 44 KiB a correlated
/// `EXISTS`, measured — so a declared debug thread answers 215 `+` terms, 78 nested
/// calls and 25 correlated `EXISTS` levels, and an undeclared one 172, 63 and 21.
#[cfg(all(not(target_arch = "wasm32"), debug_assertions))]
pub const DEFAULT_MAX_DEPTH: usize = 256;
/// The wasm limit, bounded by the **host engine's** native stack, which a module can
/// neither measure nor raise.
///
/// Sized against both of V8's tiers on its default stack (Deno 2.8), at two thirds of
/// the shortest edge measured with no limit over **every arm of the parser's operator
/// loop** — each chain-forming node it builds around the chain so far — in a `SELECT`, a
/// `WHERE`, a `CHECK`, a column `DEFAULT` and an index expression: under `--no-liftoff` an
/// `IN (SELECT …)` chain ran out of V8's stack first, at 3 315 levels, and under Liftoff
/// an `IN (…)` chain in a `CHECK`, at 3 010. An arm a walker handled from its big
/// dispatch frame instead of its lean one costs several times the others: the `IN`,
/// `IN (SELECT …)`, `ANY`/`ALL` and typmod-cast arms did until each walker took them in
/// its lean frame (`Expr::chain_operands` names the set), and ran out at 1 296, 1 295,
/// 1 824 and 1 364 levels under `--no-liftoff`, inside this limit. The plan chains ran out
/// later (a CTE chain at 1 500 CTEs, two levels each). The one node the loop builds that
/// is not chain-forming — a `::regclass` target's string-function node — is charged
/// [`DISPATCH_LEVEL_WEIGHT`] a level instead, and answers 665 repetitions. The
/// linear-memory shadow stack (`crates/pglet_wasm/build.rs`) is not the binding one: every
/// edge measured was V8's `RangeError`. `tests-web/integration/depth_edge.ts` drives every
/// kind the loop builds to this limit under both tiers, and `parser::operator_loop_arms`
/// fails when a kind it reads from the builders is driven by no arm there.
#[cfg(target_arch = "wasm32")]
pub const DEFAULT_MAX_DEPTH: usize = 2000;

/// The limit a thread starts with when its stack is not known (the `stack` module's
/// third case).
///
/// It is sized with the byte check's budget there to fit the smallest stack a thread
/// has by default on the target — the arithmetic is at [`DEFAULT_STACK_BUDGET`]. Off
/// Windows that is a Rust-spawned thread's 2 MiB, which [`DEFAULT_MAX_DEPTH`] is sized
/// for already. A thread whose limit a host set explicitly ([`set_max_depth`])
/// keeps it.
#[cfg(all(not(target_arch = "wasm32"), not(windows)))]
pub const FALLBACK_MAX_DEPTH: usize = DEFAULT_MAX_DEPTH;
/// On Windows: a 1 MiB main thread's.
#[cfg(all(windows, not(debug_assertions)))]
pub const FALLBACK_MAX_DEPTH: usize = 1000;
/// See the release constant.
#[cfg(all(windows, debug_assertions))]
pub const FALLBACK_MAX_DEPTH: usize = 128;

/// The stack the native server gives each connection thread — reserved address space,
/// committed only as a statement actually reaches it. A debug build's frames are
/// about eight times a release build's, so its reservation is too.
#[cfg(not(debug_assertions))]
pub const SERVER_THREAD_STACK: usize = 64 * 1024 * 1024;
/// See the release constant.
#[cfg(debug_assertions)]
pub const SERVER_THREAD_STACK: usize = 512 * 1024 * 1024;

/// The limit the native server sets on its connection threads.
///
/// Their stack is [`SERVER_THREAD_STACK`], which the server declares to the byte check
/// ([`set_thread_stack`]). The limit is the same in both profiles, so a statement's
/// verdict does not depend on which build served it; its reserve is 2.4 MiB (release)
/// and 17.3 MiB (debug) of the reservation.
pub const SERVER_MAX_DEPTH: usize = 10_000;

/// What one plan level under an expression — a subquery's plan — counts for against
/// the limit, where every other level counts `1`.
///
/// The limit bounds the walks that cannot take the byte check ([`stack_reserve`]), and
/// the dearest of them is `Clone`: 209 B an expression level and 975 B a plan level in
/// a release build (1 649 and 6 328 debug), measured on the `+` and `UNION ALL` chains.
/// An expression is cloned where no one site can be guarded, and cloning it clones any
/// plan under it, so such a plan level is charged the ratio of the two, rounded up. A
/// plan not under an expression is cloned only at the sites that check the room first
/// ([`check_stack_room`]), and counts `1`.
#[cfg(not(debug_assertions))]
pub const EMBEDDED_PLAN_LEVEL_WEIGHT: usize = 5;
/// See the release constant: 6 328 / 1 649, rounded up.
#[cfg(debug_assertions)]
pub const EMBEDDED_PLAN_LEVEL_WEIGHT: usize = 4;

/// What one level of an expression node **no walker takes in its lean frame** counts for
/// against the limit, where a chain-forming node ([`Expr::chain_operands`]) and a leaf
/// count `1`.
///
/// Every walker recurses through the chain-forming nodes from a lean frame of its own,
/// and hands every other node with children to its big dispatch frame, which costs
/// several times as much a level. Natively the byte check bounds those frames wherever
/// they recur, and the weight is `1`. On wasm the count is the whole bound, and a node
/// that recurs through the big frames without parser nesting ran out of V8's stack inside
/// the limit: a `::regclass` chain — `parser::expr`'s cast target builds a string-function
/// node for it, not a cast — at 1 394 levels under `--no-liftoff`, 1 490 under Liftoff,
/// where the chain-forming nodes' shortest edge is 3 010. Charged three a level, such a
/// chain is refused at 666 repetitions, which that edge holds more than twice over; and
/// the rule is the node's kind, not a list of arms, so a node any future arm builds is
/// charged unless it is chain-forming.
#[cfg(target_arch = "wasm32")]
pub const DISPATCH_LEVEL_WEIGHT: usize = 3;
/// See the wasm constant: natively the byte check bounds the dispatch frames.
#[cfg(not(target_arch = "wasm32"))]
pub const DISPATCH_LEVEL_WEIGHT: usize = 1;

/// What one level of `expr` counts for against the limit: [`DISPATCH_LEVEL_WEIGHT`] for a
/// node with children that is not chain-forming, `1` for every other.
#[must_use]
pub fn expr_level_weight(expr: &Expr) -> usize {
    if DISPATCH_LEVEL_WEIGHT == 1 || expr.chain_operands().is_some() {
        return 1;
    }
    let mut dispatched = false;
    expr.for_each_child(|_| dispatched = true);
    if dispatched { DISPATCH_LEVEL_WEIGHT } else { 1 }
}

/// What one level of `node` counts for: [`expr_level_weight`] for an expression, `1`
/// for a plan (an embedded plan's weight is its parent's to decide, in [`tree_depth`]).
fn node_weight(node: Node<'_>) -> usize {
    match node {
        Node::Expr(expr) => expr_level_weight(expr),
        Node::Plan(_) => 1,
    }
}

thread_local! {
    /// The limit a host set on this thread, `None` for the default.
    static MAX_DEPTH: Cell<Option<usize>> = const { Cell::new(None) };
}

/// This thread's limit: the one a host set ([`set_max_depth`]), else
/// [`DEFAULT_MAX_DEPTH`] — or, natively, [`FALLBACK_MAX_DEPTH`] on a thread whose stack
/// cannot be learned.
#[must_use]
pub fn max_depth() -> usize {
    MAX_DEPTH.with(Cell::get).unwrap_or_else(default_max_depth)
}

#[cfg(not(target_arch = "wasm32"))]
fn default_max_depth() -> usize {
    if stack::known() {
        DEFAULT_MAX_DEPTH
    } else {
        FALLBACK_MAX_DEPTH
    }
}

#[cfg(target_arch = "wasm32")]
const fn default_max_depth() -> usize {
    DEFAULT_MAX_DEPTH
}

/// Set this thread's limit, returning the previous one.
///
/// A host calls it on a thread whose stack it sized; raising it on a thread whose stack
/// is smaller than the limit assumes is how a statement overflows that stack — on a
/// native thread, the stack must hold [`stack_reserve`] at the new limit below the
/// byte check's own.
pub fn set_max_depth(depth: usize) -> usize {
    let previous = max_depth();
    MAX_DEPTH.with(|cell| cell.set(Some(depth)));
    // The reserve below the byte check's limit grows with the count (`stack_reserve`).
    #[cfg(not(target_arch = "wasm32"))]
    stack::refresh();
    previous
}

#[cfg(not(target_arch = "wasm32"))]
mod stack;
#[cfg(not(target_arch = "wasm32"))]
pub use stack::{
    DEFAULT_STACK_BUDGET, INFALLIBLE_LEVEL_BYTES, STACK_SLOP, StackEntry, enter as enter_stack,
    reserve as stack_reserve, set_thread_stack, thread_bounds as thread_stack_bounds,
};

/// Refuse `54001` unless a walk needing `bytes` more stack fits above this thread's
/// limit.
///
/// It is for an infallible walk dearer a level than the reserve provides for
/// ([`INFALLIBLE_LEVEL_BYTES`]). On wasm it checks nothing.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) when it would not fit.
#[cfg_attr(target_arch = "wasm32", allow(clippy::missing_const_for_fn))]
pub fn check_stack_room(bytes: usize) -> Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    return stack::check_room(bytes);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = bytes;
        Ok(())
    }
}

/// Refuse `54001` when the calling frame is within this thread's stack reserve.
///
/// It is the byte check every fallible recursion takes once a level (`stack` module
/// doc). On wasm, whose host stack cannot be observed, it checks nothing and the count
/// bound is the whole bound.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) below this thread's limit.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
#[cfg_attr(target_arch = "wasm32", allow(clippy::missing_const_for_fn))]
pub fn check_stack() -> Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    return stack::check();
    #[cfg(target_arch = "wasm32")]
    Ok(())
}

/// A host entry on wasm, where there is no byte check to scope.
#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub struct StackEntry;

/// See the native `enter_stack`; wasm has no byte check.
#[cfg(target_arch = "wasm32")]
pub const fn enter_stack(_budget: Option<usize>) -> StackEntry {
    StackEntry
}

thread_local! {
    /// How many [`EncodingLevel`]s are open on this thread.
    static ENCODING_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// Whether an [`EncodingLevel`] refused since the last [`take_encoding_refusal`].
    static ENCODING_REFUSED: Cell<bool> = const { Cell::new(false) };
}

/// One level of the catalog encoding's walk over an [`Expr`], closed when dropped.
///
/// `serde`'s derived serializer and deserializer recurse once a level like any walker,
/// and are made fallible by entering one of these at each `Expr` (`planner::expr`'s
/// `Serialize` and `Deserialize` impls).
#[derive(Debug)]
#[must_use = "the level lasts as long as the guard"]
pub struct EncodingLevel(());

impl EncodingLevel {
    /// Open a level: refused past this thread's count ([`max_depth`]) or its stack
    /// ([`check_stack`]). Refusing marks the thread ([`take_encoding_refusal`]), since
    /// the serde format carries the error back without its cause.
    ///
    /// # Errors
    ///
    /// [`Error::StackDepthExceeded`] past either bound.
    pub fn enter() -> Result<Self> {
        let depth = ENCODING_DEPTH.with(|cell| {
            let depth = cell.get() + 1;
            cell.set(depth);
            depth
        });
        let level = Self(());
        let verdict = check(depth).and_then(|()| check_stack());
        if verdict.is_err() {
            ENCODING_REFUSED.with(|cell| cell.set(true));
        }
        verdict.map(|()| level)
    }
}

impl Drop for EncodingLevel {
    fn drop(&mut self) {
        ENCODING_DEPTH.with(|cell| cell.set(cell.get().saturating_sub(1)));
    }
}

/// Whether an encoding walk on this thread was refused since the last call — what a
/// caller of `postcard` asks when it fails, to answer `54001` rather than a corrupt
/// value.
pub fn take_encoding_refusal() -> bool {
    ENCODING_REFUSED.with(|cell| cell.replace(false))
}

/// What `Clone` costs a plan level, rounded up from what a `UNION ALL` chain measured —
/// 975–976 B (release) and 6 328–6 339 B (debug) across the recorded measurements — the
/// room [`clone_plan`] asks for a level.
#[cfg(not(debug_assertions))]
pub const PLAN_CLONE_LEVEL_BYTES: usize = 1024;
/// See the release constant.
#[cfg(debug_assertions)]
pub const PLAN_CLONE_LEVEL_BYTES: usize = 6656;

/// Clone `plan` where the stack has room for it.
///
/// A plan's `Clone` is a walk that cannot fail and costs more a level than the reserve
/// holds ([`EMBEDDED_PLAN_LEVEL_WEIGHT`]), so every site that clones a plan not under an
/// expression goes through here.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) where the stack lacks the room.
pub fn clone_plan(plan: &Plan) -> Result<Plan> {
    #[cfg(not(target_arch = "wasm32"))]
    check_stack_room(plan_depth(plan).saturating_mul(PLAN_CLONE_LEVEL_BYTES))?;
    Ok(plan.clone())
}

/// Refuse to store `expr` where this thread could not encode it and decode it again.
///
/// Those are the walks a commit and an open run over it, and they take the depth bounds
/// a level ([`EncodingLevel`]): an expression accepted past what they can walk would fail
/// the commit that persists it, or the open that reads it back, on the thread that wrote
/// it. The walks themselves are the measure, run where the DDL statement runs; an
/// index's build and maintenance decode its expressions again from deeper frames, and
/// refuse `54001` there.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) where either walk is refused.
pub fn check_stored_expr(expr: &Expr) -> Result<()> {
    // The same room every later statement's entry will ask for over this expression,
    // asked here — deeper in the stack than any entry — so an expression this thread
    // stores is one it can walk afterwards. Without it a `CHECK` accepted at the edge of
    // a thread's stack left the database refused on that very thread.
    let depth = expr_depth(expr);
    check_catalog_walks(depth)?;
    // One of the two doors a catalog's expressions come in by ([`STORED_DEPTH_SEEN`]).
    note_stored_depth(depth);
    let bytes = crate::planner::serialize_expr(expr)?;
    crate::planner::deserialize_expr(&bytes).map(drop)
}

/// A failed encoding walk's error: `54001` when an [`EncodingLevel`] refused, else
/// `other`.
pub fn encoding_error(other: Error) -> Error {
    if take_encoding_refusal() {
        Error::StackDepthExceeded
    } else {
        other
    }
}

/// What one level of a stored expression costs the stack when the catalog is
/// **encoded** — the walk every commit runs before it publishes — rounded up from the
/// dearest measured: 240 B (release, `-Oz`; a `coalesce` nesting, whose argument list
/// is a frame of its own) and 250 B at `-O3`, 188 B on the operator chains; 7.3 KiB in a
/// debug build. The encoding takes the byte check a level ([`EncodingLevel`]), so a
/// catalog deeper than this thread's stack has room for is refused there rather than
/// overflowing it; [`check_catalog_walks`] asks for that room before a statement runs,
/// so the refusal never falls past a commit's publish.
///
/// **The dearest level and the deepest catalog are different shapes**, which is what
/// makes one constant over a product sound and is worth stating where the constant is:
/// the `coalesce` nesting this is rounded up from is a *nesting*, and `parser::MAX_DEPTH`
/// refuses one past 128 levels, so no catalog holds more than 128 dear levels. Above
/// that every level is an operator-chain arm at 188 B. The constant covers both — the
/// shallow-and-dear 128 × 240 B and the deep-and-cheap n × 188 B.
///
/// **Which of the two an instrument should drive to test the constant is the cheap one,
/// measured** — the intuition that the dear shape is the sharper probe is wrong here,
/// and the reason is the `+ ENCODE_ENTRY_BYTES` term beside the product. The check
/// reserves `depth × this + ENCODE_ENTRY_BYTES`, so a per-level shortfall is multiplied
/// by the depth and the entry allowance absorbs it; a shape reachable only to 128 levels
/// has 128 times too little leverage on it. Bisected against
/// `crates/pglet/tests/stack_depth.rs`'s `every_statement_answers_alike_across_the_edge`
/// in a release build, one leg at a time: the **chain** leg at 1 000 levels is green at
/// 200 and red at 150, the **nesting** leg at 120 levels green at 150 and 120 and red at
/// 80. The chain leg is the sharper instrument for this constant by about 1.5×, and the
/// nesting leg is there for the shape coverage — the sweep's all-or-nothing property
/// over a `coalesce` tree's clone, restore and encode — not for the bound.
#[cfg(not(debug_assertions))]
pub const ENCODE_LEVEL_BYTES: usize = 320;
/// See the release constant.
#[cfg(debug_assertions)]
pub const ENCODE_LEVEL_BYTES: usize = 8 * 1024;

/// The stack between a host's entry and the first level of a commit's encoding — the
/// statement's own publish frames, and the catalog's containers above its expressions.
/// Measured as the intercept of the room a commit needs against the depth of the
/// catalog's deepest expression: under 2 KiB release, about 15 KiB debug.
#[cfg(not(debug_assertions))]
pub const ENCODE_ENTRY_BYTES: usize = 16 * 1024;
/// See the release constant.
#[cfg(debug_assertions)]
pub const ENCODE_ENTRY_BYTES: usize = 48 * 1024;

/// The depth of `schema`'s deepest **stored expression** — a column `DEFAULT`, a `CHECK`,
/// an index key expression or an index predicate — in [`expr_depth`]'s measure, `0` for
/// a catalog that holds none.
///
/// It is what bounds the walks a statement runs over the whole catalog
/// ([`check_catalog_walks`]), and what it claims is a **compile** gate, stated as the
/// two shapes it is made of rather than as a covering sentence. A new **field** on one
/// of the five containers this walk enters — [`Schema`], [`Table`], [`Column`],
/// [`CheckConstraint`], [`Index`] — cannot compile, because each is destructured with
/// no `..`. A new **variant** of any of the five enums it reads — [`ColumnDefault`],
/// [`IdentityGeneration`], [`IndexKey`], [`Uniqueness`] and the [`ConstraintKind`]
/// under it — cannot compile either, because each is read by an exhaustive `match`
/// with named arms and no wildcard. The enum half is the one that was missing: read
/// through `if let` and `_`, **all five** took a probe variant carrying an `Expr`
/// without a word from this function, and all five now refuse it (`E0004`), measured
/// one at a time.
///
/// What is **outside** the gate is a predicate and not a list of names: **whatever this
/// walk binds to `_`**. It reaches a struct **field** (`Schema::sequences`,
/// `Schema::enums`, `Table::foreign_keys`, and every other `_` in the body below), and
/// equally an enum **arm's payload**, which the exhaustive `match` decides the *arm* of
/// and never the value inside it — `ColumnDefault::Literal(_)`,
/// `ColumnDefault::NextVal(_)`, `IndexKey::Columns(_)`, and a field inside an arm's
/// braces, `Uniqueness::Unique { enforced: _, .. }`. Measured one at a time with
/// `cargo check -p pglet_sql`: retyping each of those four payloads to an `Expr`, and
/// adding an `Expr` field to `Sequence`, `EnumType` and `ForeignKey`, raise errors
/// elsewhere in the crate and **none** in this file — against the `E0027` this function
/// takes for a field added to `Table`, `Column` or `CheckConstraint`. So read the `_`s
/// in the body rather than any list of names; an expression grown under one of them is
/// what the gate admits.
#[must_use]
pub fn catalog_depth(schema: &Schema) -> usize {
    let Schema {
        tables,
        indexes,
        sequences: _,
        enums: _,
        generation: _,
        next_oid: _,
    } = schema;
    let mut deepest = 0;
    let mut measure = |expr: &Expr| deepest = deepest.max(expr_depth(expr));
    for table in tables.values() {
        let Table {
            name: _,
            schema: _,
            columns,
            oid: _,
            checks,
            foreign_keys: _,
        } = table;
        for column in columns {
            let Column {
                name: _,
                type_oid: _,
                nullable: _,
                position: _,
                default,
                enum_type: _,
                numeric_typmod: _,
                varchar_length: _,
                identity,
                temporal_precision: _,
            } = column;
            // Exhaustive, named, no wildcard: a new `ColumnDefault` arm must be
            // decided here before it compiles.
            match default {
                Some(ColumnDefault::Expr(expr)) => measure(expr),
                Some(
                    ColumnDefault::Literal(_)
                    | ColumnDefault::Now
                    | ColumnDefault::CurrentDate
                    | ColumnDefault::RandomUuid
                    | ColumnDefault::NextVal(_)
                    | ColumnDefault::CurrentTime
                    | ColumnDefault::TransactionTimestamp
                    | ColumnDefault::StatementTimestamp
                    | ColumnDefault::ClockTimestamp
                    | ColumnDefault::CurrentTimestamp,
                )
                | None => {}
            }
            match identity {
                Some(IdentityGeneration::Always | IdentityGeneration::ByDefault) | None => {}
            }
        }
        for check in checks {
            let CheckConstraint { name: _, expr } = check;
            measure(expr);
        }
    }
    for index in indexes.values() {
        let Index {
            name: _,
            table_name: _,
            key,
            uniqueness,
            predicate,
            nulls_not_distinct: _,
            oid: _,
        } = index;
        match key {
            IndexKey::Exprs(items) => items.iter().for_each(&mut measure),
            IndexKey::Columns(_) => {}
        }
        match uniqueness {
            Uniqueness::Unique {
                enforced: _,
                constraint,
            } => match constraint {
                Some(ConstraintKind::PrimaryKey | ConstraintKind::Unique) | None => {}
            },
            Uniqueness::NonUnique => {}
        }
        if let Some(expr) = predicate {
            measure(expr);
        }
    }
    deepest
}

/// Refuse `54001` unless this thread can walk a catalog `depth` levels deep.
///
/// `depth` is [`catalog_depth`]'s, and every host asks this at the entry of each
/// statement it runs, before anything of the statement runs.
///
/// A statement can walk the whole catalog, and most of those walks cannot fail: a
/// transaction block's snapshot is a `Clone` of it (`BEGIN`, `SAVEPOINT`), a rollback
/// drops one, and a commit **encodes** it where the store keeps it, after the statement's
/// writes are staged. A catalog is built under the bounds of the thread that built it, so
/// it can be deeper than another thread walks — the count lowered since, a narrower
/// budget, a smaller thread — and there an infallible walk overflows the stack and aborts
/// the process, and a refused encoding answered an error for a write already made. So the
/// two bounds are asked for here, up front:
///
/// - the **count**: `depth` within this thread's limit, which the byte check's reserve
///   holds an infallible walk over ([`stack_reserve`]);
/// - natively, the **room** for the encoding from this frame: `depth` ×
///   [`ENCODE_LEVEL_BYTES`] + [`ENCODE_ENTRY_BYTES`] above the byte check's limit.
///
/// An expression a statement adds is checked where it is added ([`check_stored_expr`]:
/// the encoding and the decoding, from deeper in the statement than any commit runs
/// them), so a catalog that passes here at the entry stays walkable for the rest of
/// the call.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) when either bound refuses.
pub fn check_catalog_walks(depth: usize) -> Result<()> {
    if depth == 0 {
        // A catalog with no stored expression: the walks have nothing to recurse
        // through, and no room is asked for.
        return Ok(());
    }
    check(depth)?;
    #[cfg(not(target_arch = "wasm32"))]
    stack::check_above_limit(
        depth
            .saturating_mul(ENCODE_LEVEL_BYTES)
            .saturating_add(ENCODE_ENTRY_BYTES),
    )?;
    Ok(())
}

/// The deepest stored expression this **process** has taken into a catalog: every one
/// goes through [`check_stored_expr`] where a DDL statement stores it, or through
/// [`catalog_depth`] where `catalog::deserialize_schema` reads a store back.
///
/// It is the upper bound [`CatalogDepth`] holds a catalog to without walking it. Never
/// lowered, and shared by every thread, so a bound taken from it is conservative for
/// any catalog in the process — which is what lets one walk answer for many statements.
static STORED_DEPTH_SEEN: AtomicUsize = AtomicUsize::new(0);

/// Record that a catalog in this process holds an expression `depth` levels deep
/// ([`STORED_DEPTH_SEEN`]).
pub fn note_stored_depth(depth: usize) {
    STORED_DEPTH_SEEN.fetch_max(depth, Ordering::Relaxed);
}

/// The deepest stored expression this process has taken into a catalog.
#[must_use]
pub fn stored_depth_seen() -> usize {
    STORED_DEPTH_SEEN.load(Ordering::Relaxed)
}

/// A host's record of the depth of the catalog it runs statements against — what
/// [`check_catalog_walks`] is asked at every statement's entry.
///
/// Measuring a catalog is a walk over every stored expression, and a statement is not
/// the place to pay for one. So the record answers from what it has:
///
/// - the **measure** it took, while the catalog's [`Schema::generation`] has not moved
///   since (it moves at every statement that changes the catalog's shape, and at every
///   restore that takes such a change back);
/// - else [`stored_depth_seen`], the deepest expression **any** catalog in this process
///   has taken in — an upper bound for this one, and the fast path for a thread with
///   room to spare;
/// - else — a catalog this record has never measured, which is where a host that hands
///   out a mutable catalog puts it ([`Self::forget`]) — nothing, and the walk is taken.
///
/// A bound that refuses is never the answer: the walk runs and the exact depth decides,
/// so an upper bound costs a measurement rather than a wrong refusal.
#[derive(Debug, Default)]
pub struct CatalogDepth(Cell<Option<(u64, usize)>>);

impl CatalogDepth {
    /// Refuse `54001` where this thread could not walk `schema`
    /// ([`check_catalog_walks`]).
    ///
    /// # Errors
    ///
    /// [`Error::StackDepthExceeded`] (`54001`), the catalog's exact depth having
    /// decided it.
    pub fn check(&self, schema: &Schema) -> Result<()> {
        let bound = match self.0.get() {
            Some((generation, depth)) if generation == schema.generation => Some(depth),
            Some(_) => Some(stored_depth_seen()),
            None => None,
        };
        if let Some(bound) = bound
            && check_catalog_walks(bound).is_ok()
        {
            return Ok(());
        }
        self.measure(schema)
    }

    /// The walk, where the bound could not answer — out of line, and out of the hot
    /// path's way. Inlined into it, a statement that never walks a catalog measured 7 %
    /// slower on a `CREATE TABLE` bench (78.2 against 73.2 µs over a 5-table catalog,
    /// 228.6 against 213.3 over a 150-table one), the walk itself running five times in
    /// the whole run: what costs is the code it brings into the caller under `-Oz` and
    /// one codegen unit, not the check.
    #[cold]
    #[inline(never)]
    fn measure(&self, schema: &Schema) -> Result<()> {
        let depth = catalog_depth(schema);
        self.0.set(Some((schema.generation, depth)));
        check_catalog_walks(depth)
    }

    /// Walk the catalog again at the next check, whatever the generation says — for a
    /// host that has handed out a mutable catalog, whose new expressions went through
    /// neither of [`STORED_DEPTH_SEEN`]'s doors.
    pub fn forget(&self) {
        self.0.set(None);
    }
}

/// Refuse a tree `depth` levels deep when it exceeds this thread's limit.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] (`54001`) past [`max_depth`].
pub fn check(depth: usize) -> Result<()> {
    if depth > max_depth() {
        Err(Error::StackDepthExceeded)
    } else {
        Ok(())
    }
}

/// A node of a statement tree: the two IR types a walker recurses through.
#[derive(Clone, Copy)]
enum Node<'a> {
    Plan(&'a Plan),
    Expr(&'a Expr),
}

/// The depth of `expr`: `1` for a leaf, else its level's weight more than its deepest
/// child.
///
/// The weight is [`expr_level_weight`]'s, and a subquery's plan counts as the child it
/// is to the walkers that descend into it.
#[must_use]
pub fn expr_depth(expr: &Expr) -> usize {
    // A leaf is most of what the parser finishes; answer it without the walk's stack.
    let mut leaf = !matches!(
        expr,
        Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. }
    );
    expr.for_each_child(|_| leaf = false);
    if leaf {
        1
    } else {
        tree_depth(Node::Expr(expr), std::ptr::null(), 0)
    }
}

/// The depth of `plan`, its expressions and their subqueries included.
#[must_use]
pub fn plan_depth(plan: &Plan) -> usize {
    tree_depth(Node::Plan(plan), std::ptr::null(), 0)
}

/// The depth of `expr`, one of whose subtrees is already measured.
///
/// The node at address `known` — found by address, and read only for its own level's
/// weight — counts as `known_depth` levels and is not descended into.
///
/// What keeps the parser's operator loop linear: each wrap puts the operand chain
/// built so far under one new node, and re-measuring that chain on every wrap would
/// be quadratic in its length.
#[must_use]
pub fn expr_depth_over(expr: &Expr, known: *const Expr, known_depth: usize) -> usize {
    // The arms that put the known subtree directly under the new node, answered from
    // the node's other child alone.
    let is_known = |e: &Expr| std::ptr::eq(e, known);
    match expr {
        Expr::BinaryOp { left, right, .. } | Expr::AnyAll { left, right, .. } if is_known(left) => {
            1 + known_depth.max(expr_depth(right))
        }
        Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::Cast { expr: inner, .. }
        | Expr::CastTypmod { expr: inner, .. }
        | Expr::BooleanTest { expr: inner, .. }
            if is_known(inner) =>
        {
            1 + known_depth
        }
        _ => tree_depth(Node::Expr(expr), known, known_depth),
    }
}

/// Iterative depth-first measurement with an explicit stack — the one walk over a
/// statement tree that cannot overflow however deep the tree is. `known` is as
/// [`expr_depth_over`] reads it; null when nothing is known.
fn tree_depth(root: Node<'_>, known: *const Expr, known_depth: usize) -> usize {
    let mut max = 0;
    let mut stack = WalkStack::new(root);
    while let Some((node, depth, embedded)) = stack.pop() {
        let mut push = |child| {
            // A plan under an expression — a subquery's — is a level `Clone` reaches
            // through the expression, and weighs what that level costs it.
            let embedded = embedded || matches!((node, child), (Node::Expr(_), Node::Plan(_)));
            let weight = match child {
                Node::Plan(_) if embedded => EMBEDDED_PLAN_LEVEL_WEIGHT,
                other => node_weight(other),
            };
            stack.push((child, depth + weight, embedded));
        };
        match node {
            Node::Expr(e) if std::ptr::eq(e, known) => {
                // `known_depth` counts the known node's own level, at its weight.
                max = max.max(depth - expr_level_weight(e) + known_depth);
                continue;
            }
            Node::Expr(expr) => expr_children(expr, &mut push),
            Node::Plan(plan) => plan_children(plan, &mut push),
        }
        max = max.max(depth);
    }
    max
}

/// The measurement's explicit stack: a fixed array first, so the small trees that are
/// nearly every expression the parser finishes are measured without allocating, and a
/// `Vec` only past it.
struct WalkStack<'a> {
    inline: [Option<WalkEntry<'a>>; 32],
    len: usize,
    spill: Vec<WalkEntry<'a>>,
}

/// A node on the measurement's stack: its depth, and whether it is under an expression.
type WalkEntry<'a> = (Node<'a>, usize, bool);

impl<'a> WalkStack<'a> {
    fn new(root: Node<'a>) -> Self {
        let mut inline = [None; 32];
        inline[0] = Some((root, node_weight(root), false));
        Self {
            inline,
            len: 1,
            spill: Vec::new(),
        }
    }

    fn push(&mut self, entry: WalkEntry<'a>) {
        if let Some(slot) = self.inline.get_mut(self.len) {
            *slot = Some(entry);
            self.len += 1;
        } else {
            self.spill.push(entry);
        }
    }

    fn pop(&mut self) -> Option<WalkEntry<'a>> {
        if let Some(entry) = self.spill.pop() {
            return Some(entry);
        }
        self.len = self.len.checked_sub(1)?;
        self.inline[self.len].take()
    }
}

/// Every child a walker can recurse into from `expr`: its scalar children, and the
/// sub-plan of the three plan-carrying variants [`Expr::for_each_child`] treats as a
/// leaf.
fn expr_children<'a>(expr: &'a Expr, push: &mut dyn FnMut(Node<'a>)) {
    match expr {
        Expr::Subquery { plan } | Expr::Exists { plan, .. } | Expr::InSubquery { plan, .. } => {
            push(Node::Plan(plan));
        }
        _ => {}
    }
    expr.for_each_child(|child| push(Node::Expr(child)));
}

fn push_exprs<'a>(items: &'a [Expr], push: &mut dyn FnMut(Node<'a>)) {
    for e in items {
        push(Node::Expr(e));
    }
}

/// Every sub-plan and expression `plan` carries.
///
/// **Exhaustive by design** — no wildcard arm — so a new plan variant cannot compile
/// until it states what it carries. An expression missed here is measured anyway
/// where the parser finished it; a missed *sub-plan* would leave a plan chain through
/// it unmeasured.
#[allow(clippy::too_many_lines)]
fn plan_children<'a>(plan: &'a Plan, push: &mut dyn FnMut(Node<'a>)) {
    match plan {
        Plan::Values { rows, .. } => {
            for row in rows {
                push_exprs(row, push);
            }
        }
        Plan::Scan { .. }
        | Plan::VirtualScan { .. }
        | Plan::CteScan { .. }
        | Plan::Empty
        | Plan::Noop { .. }
        | Plan::AlterTableUnperformed { .. }
        | Plan::Copy { .. }
        | Plan::DropTable { .. }
        | Plan::Truncate { .. }
        | Plan::DropIndex { .. }
        | Plan::CreateType { .. }
        | Plan::DropType { .. }
        | Plan::DropSchema { .. }
        | Plan::CreateSchema { .. }
        | Plan::AlterTableDropColumn { .. }
        | Plan::AlterTableSetNotNull { .. }
        | Plan::AlterTableAddIdentity { .. }
        | Plan::CreateSequence { .. }
        | Plan::AlterSequence { .. }
        | Plan::DropSequence { .. }
        | Plan::BeginTransaction { .. }
        | Plan::CommitTransaction
        | Plan::RollbackTransaction
        | Plan::Savepoint { .. }
        | Plan::RollbackToSavepoint { .. }
        | Plan::ReleaseSavepoint { .. } => {}
        #[cfg(feature = "index_acceleration")]
        Plan::IndexScan { key_value, .. } => push(Node::Expr(key_value)),
        #[cfg(feature = "index_acceleration")]
        Plan::IndexRangeScan { lower, upper, .. } => {
            for bound in lower.iter().chain(upper) {
                push(Node::Expr(&bound.value));
            }
        }
        #[cfg(feature = "index_acceleration")]
        Plan::GinScan { probes, guards, .. } => {
            push_exprs(probes, push);
            push_exprs(guards, push);
        }
        Plan::Filter { input, predicate } => {
            push(Node::Plan(input));
            push(Node::Expr(predicate));
        }
        Plan::Project {
            input,
            exprs: items,
            ..
        } => {
            push(Node::Plan(input));
            push_exprs(items, push);
        }
        Plan::ProjectUnnest { exprs: items } => push_exprs(items, push),
        Plan::CreateTable {
            columns, checks, ..
        } => {
            for column in columns {
                if let Some(ColumnDefault::Expr(e)) = &column.default {
                    push(Node::Expr(e));
                }
            }
            for check in checks {
                push(Node::Expr(&check.expr));
            }
        }
        Plan::CreateIndex { key, predicate, .. } => {
            if let IndexKey::Exprs(items) = key {
                push_exprs(items, push);
            }
            if let Some(e) = predicate {
                push(Node::Expr(e));
            }
        }
        Plan::AlterTableAddColumn { column, checks, .. } => {
            if let Some(ColumnDefault::Expr(e)) = &column.default {
                push(Node::Expr(e));
            }
            for check in checks {
                push(Node::Expr(&check.expr));
            }
        }
        Plan::AlterTableAddCheck { check, .. } => push(Node::Expr(&check.expr)),
        Plan::AlterTableAddConstraint { constraint, .. } => match constraint {
            AddConstraint::PrimaryKey { .. }
            | AddConstraint::Unique { .. }
            | AddConstraint::ForeignKey(_) => {}
        },
        Plan::AlterTableSetDefault { default, .. } => {
            if let Some(ColumnDefault::Expr(e)) = default {
                push(Node::Expr(e));
            }
        }
        Plan::Insert {
            rows,
            returning,
            on_conflict,
            ..
        } => {
            match rows {
                InsertRows::Values { rows, .. } => {
                    for row in rows {
                        push_exprs(row, push);
                    }
                }
                InsertRows::Select { source, .. } => push(Node::Plan(source)),
            }
            push_exprs(returning, push);
            if let Some(clause) = on_conflict {
                if let Some(target) = &clause.target {
                    push_exprs(&target.keys, push);
                    if let Some(e) = &target.predicate {
                        push(Node::Expr(e));
                    }
                }
                if let OnConflictAction::DoUpdate {
                    assignments,
                    where_predicate,
                } = &clause.action
                {
                    for (_, e) in assignments {
                        push(Node::Expr(e));
                    }
                    if let Some(e) = where_predicate {
                        push(Node::Expr(e));
                    }
                }
            }
        }
        Plan::Update {
            from,
            assignments,
            filter,
            returning,
            ..
        } => {
            if let Some(from) = from {
                push(Node::Plan(from));
            }
            for (_, e) in assignments {
                push(Node::Expr(e));
            }
            if let Some(e) = filter {
                push(Node::Expr(e));
            }
            push_exprs(returning, push);
        }
        Plan::Delete {
            filter, returning, ..
        } => {
            if let Some(e) = filter {
                push(Node::Expr(e));
            }
            push_exprs(returning, push);
        }
        Plan::Sort {
            input, order_by, ..
        } => {
            push(Node::Plan(input));
            for key in order_by {
                push(Node::Expr(&key.expr));
            }
        }
        Plan::Distinct { input }
        | Plan::JunkFilter { input, .. }
        | Plan::Limit { input, .. }
        | Plan::LockRows { input } => {
            push(Node::Plan(input));
        }
        Plan::DistinctOn { on_exprs, input } => {
            push(Node::Plan(input));
            push_exprs(on_exprs, push);
        }
        Plan::Aggregate {
            input,
            group_by,
            exprs: items,
            having,
            ..
        } => {
            push(Node::Plan(input));
            push_exprs(group_by, push);
            push_exprs(items, push);
            if let Some(e) = having {
                push(Node::Expr(e));
            }
        }
        Plan::Join {
            left, right, on, ..
        } => {
            push(Node::Plan(left));
            push(Node::Plan(right));
            push(Node::Expr(on));
        }
        Plan::SetOp { left, right, .. } => {
            push(Node::Plan(left));
            push(Node::Plan(right));
        }
        Plan::SubqueryScan { subquery, .. } => push(Node::Plan(subquery)),
        Plan::With { ctes, body } => {
            for cte in ctes {
                for sub in cte.plans() {
                    push(Node::Plan(sub));
                }
            }
            push(Node::Plan(body));
        }
        Plan::FunctionScan { args, .. } => push_exprs(args, push),
        Plan::Window {
            input,
            window_exprs,
            ..
        } => {
            push(Node::Plan(input));
            push_exprs(window_exprs, push);
        }
        Plan::DoBlock { body, exception } => {
            for sub in body {
                push(Node::Plan(sub));
            }
            if let Some(handler) = exception {
                for sub in &handler.handler {
                    push(Node::Plan(sub));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::Schema;
    use crate::parser::{parse, parse_expr, parse_single};
    use crate::planner::Expr;

    /// Run `f` with this thread's limit at `limit`, restoring it however `f` returns.
    fn with_limit<T>(limit: usize, f: impl FnOnce() -> T) -> T {
        let previous = super::set_max_depth(limit);
        let out = f();
        super::set_max_depth(previous);
        out
    }

    fn chain(n: usize) -> String {
        vec!["1"; n].join(" + ")
    }

    fn sqlstate<T>(result: crate::Result<T>) -> &'static str {
        result.err().map_or("ok", |e| e.sqlstate())
    }

    // The refusal is deferred to the statement's end, and outranked there by exactly the
    // errors pglet can attribute to 18.6's front end ahead of analysis
    // (`Parser::refusal_answer`): a text 18.6's raw parse certainly refuses — its brackets
    // do not nest, or 18.6's scanner refuses it as pglet's does — and an error a site
    // records as one 18.6 raises before the analysis that owes the refusal. A `42601` of
    // pglet's own narrower grammar is not one of them, whatever its shape: `SELECT … 1` and
    // `LIMIT 1+1` raise the same positioned error at the same site, and 18.6's grammar
    // refuses only the first.
    #[test]
    fn only_an_error_raised_before_analysis_outranks_a_refused_chain() {
        let schema = Schema::new();
        with_limit(50, || {
            let long = chain(100);
            let parens = format!("{}1{}", "(".repeat(130), ")".repeat(130));
            assert_eq!(sqlstate(parse(&format!("SELECT {long}"), &schema)), "54001");
            for (sql, want) in [
                // the raw parse fails: brackets
                (format!("SELECT {long} + )"), "42601"),
                (format!("SELECT {long}, (1"), "42601"),
                (format!("SELECT {long}, (1]"), "42601"),
                (format!("SELECT {long}; SELECT ("), "42601"),
                (format!("DO $$ BEGIN PERFORM {long} + ); END $$"), "42601"),
                // the raw parse fails: the scanner
                (format!("SELECT {long}, 'abc"), "42601"),
                (format!("SELECT {long}, \\ 1"), "42601"),
                // ...and not where both scanners lex the text and nothing before
                // analysis refuses it — an operator pglet implements nothing for,
                // `U&"…"` (three tokens here), a radix literal wider than `int8` (both
                // engines answer it) — but a scanner error or a bracket after one still
                // counts
                (format!("SELECT {long}, 'a' ~ 'b'"), "54001"),
                (format!("SELECT {long}, 1 & 1"), "54001"),
                (format!("SELECT {long} AS U&\"x\""), "54001"),
                (format!("SELECT {long}, 0x{}", "F".repeat(40)), "54001"),
                (format!("SELECT {long}, '{{}}'::jsonb @? '$'"), "54001"),
                (format!("SELECT {long}, 1 & 1, 'abc"), "42601"),
                (format!("SELECT {long}, 1 & 1, (1"), "42601"),
                // A `DO` body: the raw parse covers every statement of it, so a
                // `qualified_name`'s four-part refusal — or a dotted `ColId`'s — in a
                // later statement outranks an earlier statement's refusal...
                (
                    format!("DO $$ BEGIN SELECT {long}; INSERT INTO a.b.c.d VALUES (1); END $$"),
                    "42601",
                ),
                (
                    format!("DO $$ BEGIN SELECT {long}; CREATE SCHEMA a.b.c.d; END $$"),
                    "42601",
                ),
                // ...while an `any_name`'s is raised only when its statement runs, and
                // `transformColumnDefinition`'s when its statement is analysed — each
                // after the earlier statement has answered `54001`
                (
                    format!("DO $$ BEGIN SELECT {long}; DROP TABLE a.b.c.d; END $$"),
                    "54001",
                ),
                (
                    format!("DO $$ BEGIN SELECT {long}; DROP INDEX a.b.c.d; END $$"),
                    "54001",
                ),
                (
                    format!("DO $$ BEGIN SELECT {long}; CREATE TYPE a.b.c.d AS ENUM ('x'); END $$"),
                    "54001",
                ),
                (
                    format!(
                        "DO $$ BEGIN SELECT {long}; CREATE TABLE r (x int DEFAULT 1 DEFAULT 2); END $$"
                    ),
                    "54001",
                ),
                (
                    format!(
                        "DO $$ BEGIN SELECT {long}; CREATE TABLE r (x int DEFAULT 1 GENERATED ALWAYS AS IDENTITY); END $$"
                    ),
                    "54001",
                ),
                (
                    format!(
                        "DO $$ BEGIN SELECT {long}; CREATE TABLE r (x int GENERATED ALWAYS AS IDENTITY DEFAULT 1); END $$"
                    ),
                    "54001",
                ),
                (
                    format!(
                        "DO $$ BEGIN SELECT {long}; CREATE TABLE r (x int GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME a.b.c.d)); END $$"
                    ),
                    "54001",
                ),
                // ...and within its own statement it still outranks
                (
                    format!(
                        "CREATE TABLE r (y int DEFAULT {long}, x int GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME a.b.c.d))"
                    ),
                    "42601",
                ),
                // `gram.y`'s own refusals, and `transformColumnDefinition`'s
                (format!("(SELECT {long} ORDER BY 1) ORDER BY 1"), "42601"),
                (format!("(SELECT {long} LIMIT 1) LIMIT 2"), "42601"),
                (format!("SELECT {long} FROM a.b.c.d"), "42601"),
                (format!("SELECT {long}, CAST(1 AS float(0))"), "22023"),
                (
                    format!("CREATE TABLE md (c int DEFAULT {long} DEFAULT 7)"),
                    "42601",
                ),
                (
                    format!("CREATE TABLE md (c int DEFAULT {long} GENERATED ALWAYS AS IDENTITY)"),
                    "42601",
                ),
                // pglet's own refusals, and grammar errors it cannot attribute
                (format!("SELECT {long} FROM WHERE"), "54001"),
                (format!("SELECT {long} 1"), "54001"),
                (format!("SELECT {long} LIMIT 1+1"), "54001"),
                (format!("SELECT {long}, 'a' IS NORMALIZED"), "54001"),
                (format!("SELECT {long}, {parens}"), "54001"),
                (format!("SELECT {long} FROM a.b.c"), "54001"),
            ] {
                assert_eq!(sqlstate(parse(&sql, &schema)), want, "{sql}");
            }
            // A set-operation chain is refused on the spot, not deferred, and the raw
            // parse still outranks it: nothing after the refusal is read, so only the
            // text itself can say 18.6's raw parse fails.
            let arms = vec!["SELECT 1"; 100].join(" UNION ALL ");
            assert_eq!(sqlstate(parse(&format!("{arms} )"), &schema)), "42601");
            assert_eq!(sqlstate(parse(&format!("{arms} 1"), &schema)), "54001");
            assert_eq!(sqlstate(parse_expr(&long, &schema)), "54001");
            assert_eq!(sqlstate(parse_expr(&format!("{long} )"), &schema)), "42601");
            assert_eq!(sqlstate(parse_expr(&format!("{long} 1"), &schema)), "54001");
            // `Parse`: 18.6 counts the statements before analysing either.
            let single = |sql: &str| sqlstate(parse_single(sql, &schema));
            assert_eq!(single(&format!("SELECT {long}; SELECT 1")), "42601");
            assert_eq!(single(&format!("SELECT {long};")), "54001");
            // ...and counts what it counts: a piece holding no token is no statement.
            assert_eq!(single(&format!("SELECT {long}; -- c")), "54001");
            assert_eq!(single(&format!("SELECT {long}; /* c */ ;")), "54001");
            // Inside the limit the chain is kept as written.
            assert!(matches!(
                parse_expr(&chain(40), &schema),
                Ok(Expr::BinaryOp { .. })
            ));
        });
    }
}
