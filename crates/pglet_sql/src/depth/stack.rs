//! The byte check: 18.6's `check_stack_depth`, native only.
//!
//! A checkpoint compares the address of one of its own locals with this thread's
//! **limit** — the lowest address recursion may reach — and refuses `54001` below it.
//! The limit is the thread's stack low end plus a reserve ([`reserve`]), and the low end
//! is learned once per thread:
//!
//! - declared by the host that sized the thread ([`set_thread_stack`] — the server's
//!   connection threads);
//! - else, on Linux, the main thread's: its `[stack]` in `/proc/self/maps` is its own
//!   mapping, and its low end is its top less the `RLIMIT_STACK` soft limit
//!   (`/proc/self/limits`);
//! - else unknown, and the limit is relative instead: a budget of stack below the
//!   outermost host entry ([`enter`]) — [`DEFAULT_STACK_BUDGET`], or the one a caller
//!   gave the entry — and the count a thread starts with shrinks to
//!   [`super::FALLBACK_MAX_DEPTH`], so the two fit the smallest stack a caller's thread
//!   has by default on any target (the arithmetic is at [`DEFAULT_STACK_BUDGET`]).
//!
//! **No other thread's stack is learned from `/proc/self/maps`, because the mapping a
//! thread runs in is not its own.** A thread's stack is an anonymous mapping like the
//! next, and the kernel merges two adjacent ones of the same protection: a thread
//! created without a guard page, placed directly above a guarded one, shares one mapping
//! with it, and the guard below that mapping is the neighbour's. Measured: a 2 MiB
//! guardless thread over an 8 MiB guarded one read as one 10 MiB stack with a guard
//! below, and a `CTE` chain answered 1 297 (882 on its real 2 MiB) by writing through
//! into the neighbour, `SIGSEGV`. Nothing in `/proc` says which thread a mapping's low
//! end belongs to. What the mapping does prove is a **floor** — no thread's stack
//! reaches below the mapping it runs in — so the limit is never below the floor plus
//! the reserve, which is what stops a smaller-than-assumed thread with a guard of its
//! own (`RUST_MIN_STACK=524288`) short of it.
//!
//! A known stack is a range, and a checkpoint outside it — a thread that has switched to
//! another stack since (a fiber, `stacker`) — learns again rather than measuring against
//! the other stack's end. (Reasoned, not measured: no in-tree instrument switches
//! stacks.)
//!
//! A thread with no known stack or floor that no host entry covers runs with no byte
//! check at all; the count bound ([`super::max_depth`]) is what protects it.

use std::cell::Cell;

use crate::{Error, Result};

/// Stack a statement keeps free below its deepest checkpoint beyond what an infallible
/// walk at the count limit needs.
///
/// It holds the frames a walker runs between two checkpoints and
/// the leaf calls below the last one. 18.6's `STACK_DEPTH_SLOP` is 512 KiB for the same
/// purpose; this half of the reserve is 256 KiB because the infallible walks, which C
/// has no counterpart of, are reserved for separately ([`INFALLIBLE_LEVEL_BYTES`]).
pub const STACK_SLOP: usize = 256 * 1024;

/// The most stack one tree level of an **infallible** walker costs — `Clone`, `Drop`,
/// `PartialEq`, `infer_type`, the plan's `output_columns`, the `contains_*`/`collect_*`
/// visitors.
///
/// None of them can take a checkpoint, so the reserve holds one of them run over a whole
/// tree at the count limit. Measured over `+` and `UNION ALL`/`JOIN` chains on threads of
/// known stack (the stack over the deepest chain each walker finished), release: `Clone`
/// 209 B a level is the dearest (`push_level_relations` 205, `output_columns` 176,
/// `contains_aggregate` 111, `infer_type` 63, `PartialEq` and `Drop` 47). A plan level
/// under an expression is charged more (`super::EMBEDDED_PLAN_LEVEL_WEIGHT`), and a
/// plan's own `Clone` checks its room (`super::clone_plan`).
#[cfg(not(debug_assertions))]
pub const INFALLIBLE_LEVEL_BYTES: usize = 224;
/// See the release constant. Debug: `Clone` 1 649 B (`output_columns` 1 300,
/// `push_level_relations` 1 290, `PartialEq` 1 137).
#[cfg(debug_assertions)]
pub const INFALLIBLE_LEVEL_BYTES: usize = 1792;

/// The stack a statement may use below the outermost host entry ([`enter`]) on a thread
/// whose own stack is not known — every thread but a declared one and a Linux main
/// thread.
///
/// It and the reserve below it ([`reserve`], at [`super::FALLBACK_MAX_DEPTH`]) fit the
/// smallest stack a caller's thread has **by default** on the target. A thread Rust
/// spawns has 2 MiB on unix and Windows alike (std's `DEFAULT_MIN_STACK_SIZE`,
/// `library/std/src/sys/thread/{unix,windows}.rs`); a main thread's size is the
/// platform's — std says it "is *not* determined by Rust" — and the smallest of those is
/// taken to be Windows' 1 MiB (a linker default; nothing in this workspace states it).
/// Of a Windows thread's reservation std keeps 20 KiB back for its stack-overflow
/// handler (`SetThreadStackGuarantee(0x5000)`, `library/std/src/sys/pal/windows/
/// stack_overflow.rs`) and the system a 4 KiB guard page, so 1 000 KiB of a 1 MiB main
/// thread is usable. In KiB, with 128 KiB assumed for the caller's frames above the entry
/// and [`STACK_SLOP`]'s 256:
///
/// - off Windows, a 2 048 KiB stack holds 128 + 1 024 (this) + 256 + the count's
///   infallible walks — 2 600 × 224 B = 569 release, 256 × 1 792 B = 448 debug:
///   1 977 and 1 856;
/// - on Windows, the 1 000 usable KiB hold 128 + 352 (this) + 256 + 1 000 × 224 B = 219
///   release, 128 × 1 792 B = 224 debug: 955 and 960, 40 KiB to spare.
///
/// A thread smaller than its target's default, or a caller deeper above the entry, sets
/// a budget and a count of its own (`pglet::Database::set_stack_budget`,
/// `pglet::set_thread_max_depth`) or declares its stack (`pglet::set_thread_stack`).
#[cfg(not(windows))]
pub const DEFAULT_STACK_BUDGET: usize = 1024 * 1024;
/// See the constant off Windows.
#[cfg(windows)]
pub const DEFAULT_STACK_BUDGET: usize = 352 * 1024;

/// What the top of a thread's stack holds above the first frame its entry closure
/// runs in — glibc's thread descriptor and static TLS, std's start glue — assumed not
/// to exceed this. Only [`set_thread_stack`] reads it.
const TOP_OF_STACK: usize = 64 * 1024;

/// The gap Linux keeps between a growing main-thread `[stack]` and the mapping below it
/// (`stack_guard_gap`, 256 pages by default).
#[cfg(target_os = "linux")]
const STACK_GUARD_GAP: usize = 1024 * 1024;

/// The limit before this thread has learned one: every address is below it, so the
/// first checkpoint takes the slow path and learns.
const UNLEARNED: usize = usize::MAX;

/// A host entry's recorded state: its stack address and the budget it was given.
#[derive(Clone, Copy)]
struct Entry {
    base: usize,
    budget: Option<usize>,
}

thread_local! {
    /// The lowest address a checkpoint lets recursion reach; `0` checks nothing.
    static LIMIT: Cell<usize> = const { Cell::new(UNLEARNED) };
    /// This thread's stack low end, `0` when unknown.
    static LOW: Cell<usize> = const { Cell::new(0) };
    /// The lowest address this thread's stack can reach — the start of the mapping it
    /// runs in, or `LOW` — `0` when unknown.
    static FLOOR: Cell<usize> = const { Cell::new(0) };
    /// The end of the stack `FLOOR` was learned for, exclusive — `usize::MAX` when
    /// nothing was: a checkpoint at or above it runs on another stack, and learns again.
    static TOP: Cell<usize> = const { Cell::new(usize::MAX) };
    /// The outermost host entry in force on this thread.
    static ENTRY: Cell<Option<Entry>> = const { Cell::new(None) };
}

/// The address of a local of the calling frame.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn here() -> usize {
    let marker = 0_u8;
    std::ptr::from_ref(std::hint::black_box(&marker)).addr()
}

/// The stack kept free below the limit: [`STACK_SLOP`] plus an infallible walk over a
/// tree at this thread's count limit.
pub fn reserve() -> usize {
    STACK_SLOP.saturating_add(super::max_depth().saturating_mul(INFALLIBLE_LEVEL_BYTES))
}

/// Refuse `54001` when this frame is below this thread's limit.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] below the limit.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
pub fn check() -> Result<()> {
    let here = here();
    if here < LIMIT.with(Cell::get) || here >= TOP.with(Cell::get) {
        return slow(here);
    }
    Ok(())
}

/// Refuse `54001` unless a walk needing `bytes` more stack than the calling frame fits.
///
/// It is the check an **infallible** walk too dear for the reserve takes once, before
/// it starts: the walk is the one the reserve would have held, so on a stack learned
/// from the OS it may use the reserve down to [`STACK_SLOP`]; under a budget it must
/// end above the limit.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] when it would not.
pub fn check_room(bytes: usize) -> Result<()> {
    check()?;
    let limit = LIMIT.with(Cell::get);
    let low = LOW.with(Cell::get);
    let floor = if low != 0 && limit == low.saturating_add(reserve()) {
        low.saturating_add(STACK_SLOP)
    } else {
        limit
    };
    if here().saturating_sub(bytes) < floor {
        return Err(Error::StackDepthExceeded);
    }
    Ok(())
}

/// Refuse `54001` unless a **fallible** walk needing `bytes` more stack than the calling
/// frame stays above the limit — the check a host runs at a statement's entry for a
/// walk the statement will run later from about as high ([`super::check_catalog_walks`]):
/// that walk takes [`check`] a level, and would refuse once below the limit.
///
/// # Errors
///
/// [`Error::StackDepthExceeded`] when it would not.
pub fn check_above_limit(bytes: usize) -> Result<()> {
    check()?;
    if here().saturating_sub(bytes) < LIMIT.with(Cell::get) {
        return Err(Error::StackDepthExceeded);
    }
    Ok(())
}

/// The first checkpoint on a thread learns its limit, and so does one on a stack other
/// than the one learned; every other call here is a refusal.
#[cold]
#[inline(never)]
fn slow(here: usize) -> Result<()> {
    let floor = FLOOR.with(Cell::get);
    let switched = floor != 0 && !(floor..TOP.with(Cell::get)).contains(&here);
    if LIMIT.with(Cell::get) == UNLEARNED || switched {
        if switched {
            LOW.with(|cell| cell.set(0));
            FLOOR.with(|cell| cell.set(0));
        }
        learn();
        let limit = LIMIT.with(Cell::get);
        if here >= limit && here < TOP.with(Cell::get) {
            return Ok(());
        }
    }
    Err(Error::StackDepthExceeded)
}

/// Learn what the OS says of this thread's stack, unless it was declared.
fn learn() {
    if LOW.with(Cell::get) == 0 {
        let stack = os_stack(here());
        LOW.with(|cell| cell.set(stack.and_then(|s| s.own_low).unwrap_or(0)));
        FLOOR.with(|cell| cell.set(stack.map_or(0, |s| s.floor)));
        TOP.with(|cell| cell.set(stack.map_or(usize::MAX, |s| s.top)));
    }
    LIMIT.with(|cell| cell.set(0));
    refresh();
}

/// Whether this thread's stack is known — declared, or the main thread's on Linux —
/// learning what the OS says first if no checkpoint has yet.
pub(super) fn known() -> bool {
    if LIMIT.with(Cell::get) == UNLEARNED {
        learn();
    }
    LOW.with(Cell::get) != 0
}

/// Recompute the limit from what this thread knows: the known low end plus the reserve,
/// raised to an entry's explicit budget where that is smaller — a budget can narrow the
/// known stack, never widen it — and [`DEFAULT_STACK_BUDGET`] below the entry where no
/// low end is known; never below the floor plus the reserve. Leaves an unlearned thread
/// unlearned.
pub(super) fn refresh() {
    if LIMIT.with(Cell::get) == UNLEARNED {
        return;
    }
    let reserve = reserve();
    let low = LOW.with(Cell::get);
    let known = (low != 0).then(|| low.saturating_add(reserve));
    let floor = FLOOR.with(Cell::get);
    // An entry made on another stack says nothing about this one.
    let top = TOP.with(Cell::get);
    let entry = ENTRY
        .with(Cell::get)
        .filter(|entry| floor == 0 || (floor..top).contains(&entry.base));
    let limit = match entry {
        Some(Entry {
            base,
            budget: Some(budget),
        }) => {
            let asked = base.saturating_sub(budget);
            known.map_or(asked, |known| known.max(asked))
        }
        Some(Entry { base, budget: None }) => {
            known.unwrap_or_else(|| base.saturating_sub(DEFAULT_STACK_BUDGET))
        }
        None => known.unwrap_or(0),
    };
    let floored = if floor == 0 {
        limit
    } else {
        limit.max(floor.saturating_add(reserve))
    };
    LIMIT.with(|cell| cell.set(floored));
}

/// What this thread knows of its own stack: its low end, its floor and its top.
///
/// `0`, `0` and `usize::MAX` where unknown. It is the part of [`check`]'s limit that is
/// the thread's, beside the count's reserve and a host entry's own base and budget
/// ([`enter`]). [`set_thread_stack`] moves it, and so does running on another thread.
#[must_use]
pub fn thread_bounds() -> (usize, usize, usize) {
    (
        LOW.with(Cell::get),
        FLOOR.with(Cell::get),
        TOP.with(Cell::get),
    )
}

/// Declare this thread's stack: `bytes` in all, the calling frame near its top. A host
/// that sized the thread calls it first thing.
///
/// **A declaration is held to what the kernel shows, never trusted past it.** The
/// mapping the thread runs in is a floor no stack reaches below (the module doc), so
/// where it starts above the declared low end the declaration overstates the stack, and
/// the mapping's start is the low end: a 2 MiB thread declared 4 MiB aborted on a CTE
/// chain at 2 048, its reserve measured from a low end 2 MiB below the real one. Where
/// the mapping reaches lower than declared, the declaration stands — the mapping may
/// hold a neighbour's stack too, and only the caller knows which part is this thread's.
///
/// What stays the caller's to get right: a declaration larger than the stack where the
/// mapping cannot show it — off Linux, where no mapping is read, and on a thread whose
/// mapping has merged with a neighbour's below it (a guardless thread directly above
/// another), whose start is the neighbour's.
pub fn set_thread_stack(bytes: usize) {
    let top = here();
    let declared = top.saturating_sub(bytes).saturating_add(TOP_OF_STACK);
    let low = os_stack(top).map_or(declared, |mapping| declared.max(mapping.floor));
    LOW.with(|cell| cell.set(low));
    FLOOR.with(|cell| cell.set(low));
    TOP.with(|cell| cell.set(top.saturating_add(TOP_OF_STACK)));
    LIMIT.with(|cell| cell.set(0));
    refresh();
}

/// A host entry in force: restores the thread's previous entry state when dropped.
#[derive(Debug)]
#[must_use = "the entry lasts as long as the guard"]
pub struct StackEntry {
    outermost: bool,
}

/// Enter a host entry point.
///
/// The statement's stack is measured from here when the thread's own stack is unknown,
/// or always when `budget` is given. Only the outermost
/// entry on a thread records; a nested one — a host calling back into itself — inherits
/// it.
pub fn enter(budget: Option<usize>) -> StackEntry {
    if ENTRY.with(Cell::get).is_some() {
        return StackEntry { outermost: false };
    }
    if LIMIT.with(Cell::get) == UNLEARNED {
        learn();
    }
    ENTRY.with(|cell| {
        cell.set(Some(Entry {
            base: here(),
            budget,
        }));
    });
    refresh();
    StackEntry { outermost: true }
}

impl Drop for StackEntry {
    fn drop(&mut self) {
        if self.outermost {
            ENTRY.with(|cell| cell.set(None));
            refresh();
        }
    }
}

/// What `/proc/self/maps` says of the stack a frame at `sp` runs on.
#[derive(Clone, Copy)]
struct OsStack {
    /// The thread's own low end, where the mapping is the thread's own — the main
    /// thread's `[stack]` — and `None` for any other.
    own_low: Option<usize>,
    /// The lowest address the stack can reach: the mapping's start (the main thread's
    /// own low end, for a `[stack]` that grows on demand).
    floor: usize,
    /// The mapping's end, exclusive.
    top: usize,
}

/// What `/proc/self/maps` says of the stack a frame at `sp` runs on, or `None` where it
/// cannot be read.
#[cfg(target_os = "linux")]
fn os_stack(sp: usize) -> Option<OsStack> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let mut previous_end: Option<usize> = None;
    for line in maps.lines() {
        let mut fields = line.split_ascii_whitespace();
        let (start, end) = fields.next()?.split_once('-')?;
        let start = usize::from_str_radix(start, 16).ok()?;
        let end = usize::from_str_radix(end, 16).ok()?;
        if (start..end).contains(&sp) {
            if line.ends_with("[stack]") {
                // The main thread's stack grows down to its rlimit, and no closer to the
                // mapping below than the guard gap.
                let low = end.checked_sub(rlimit_stack()?)?;
                let gap = previous_end.map_or(0, |prev_end| prev_end + STACK_GUARD_GAP);
                let low = low.max(gap);
                return Some(OsStack {
                    own_low: Some(low),
                    floor: low,
                    top: end,
                });
            }
            // Another thread's stack: a floor, not its low end (the module doc).
            return Some(OsStack {
                own_low: None,
                floor: start,
                top: end,
            });
        }
        previous_end = Some(end);
    }
    None
}

/// The `RLIMIT_STACK` soft limit in bytes, `None` when unlimited or unreadable.
#[cfg(target_os = "linux")]
fn rlimit_stack() -> Option<usize> {
    let limits = std::fs::read_to_string("/proc/self/limits").ok()?;
    let line = limits.lines().find(|l| l.starts_with("Max stack size"))?;
    line.split_ascii_whitespace().nth(3)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
const fn os_stack(_sp: usize) -> Option<OsStack> {
    None
}
