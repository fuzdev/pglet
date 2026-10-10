//! Niladic impure builtins (clock, RNG) backing the `now()` family,
//! `gen_random_uuid()` and `random()`.
//!
//! The native build uses the OS clock (`time`) and OS RNG (`getrandom`); the wasm
//! build sources the same capabilities from the JS host (`Date.now` for the clock,
//! `Math.random` for the RNG). The native path stays the conformance target (the
//! differential harness, native consumers, durability); the wasm path is best-effort
//! (`Math.random` is not crypto-grade), enough to serve the embedded/test backend.
//!
//! # Which clock a spelling reads
//!
//! 18.6 keeps two instants beside the wall clock, and so does this module ([`Clock`]):
//!
//! - **the statement's start** — 18.6's `SetCurrentStatementStartTimestamp`, taken as a
//!   client message arrives. A host marks the arrival with [`start_statement`], and
//!   `statement_timestamp()` answers the instant;
//! - **the transaction's start** — 18.6's `xactStartTimestamp`, which is the statement
//!   start of the statement that began the transaction. An open block carries its own
//!   (`SessionSettings::begin_block` takes the statement start its `BEGIN` ran under),
//!   and with no block open it is the statement's. `now()`, `transaction_timestamp()`,
//!   `CURRENT_TIMESTAMP`, `LOCALTIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME` and
//!   `LOCALTIME` answer it;
//! - **the wall clock** — `clock_timestamp()`, read at each call.
//!
//! **On wasm the statement's start is read where the statement first needs it**, not
//! where the host marks its arrival: most statements read no clock, and a read there is
//! a call out to the JS host (measured at about a sixth of a `SELECT 1`). The
//! statement's own reads agree with a start taken at the arrival: a read of the wall
//! clock takes the statement's start first, and a read of that clock is never earlier
//! than the one before it while the JS clock has not been set back between the two
//! ([`wall_clock`]'s step — a microsecond later through the first reads of one
//! millisecond, equal from its tie point on), so the start is not later than a `clock_timestamp()`
//! of the same statement, and a `BEGIN` takes it as it opens the block. The native
//! clock has no step — two reads a few nanoseconds apart are one microsecond — so
//! there the start is read at the arrival, as 18.6 reads it, and a `clock_timestamp()`
//! the statement goes on to evaluate is later.
//!
//! Both instants are ambient — thread-locals, for the reason the session is
//! (`SessionSettings::install_ambient`): a `DEFAULT`, a `CHECK` and a back-fill
//! evaluate at sites handed a row and nothing else. So a route that runs a statement
//! without [`start_statement`] reads the start of the statement before it — and a
//! thread that never calls it (a test driving the executor directly) keeps the first
//! instant it reads. The hosts' entry points are few, and each host's tests drive
//! every one of its own.

#[cfg(not(target_arch = "wasm32"))]
use crate::Error;
use crate::Result;
use crate::types::{PgTimestamp, Value};

/// "No instant", in the form the two starts are held in — `PgTimestamp::to_pg_micros`'
/// count, whose `i64::MIN` is no finite instant.
pub(crate) const NO_INSTANT: i64 = i64::MIN;

thread_local! {
    /// When the statement this thread is running started, once it has been read —
    /// see [`statement_start`].
    static STATEMENT_START: std::cell::Cell<i64> = const { std::cell::Cell::new(NO_INSTANT) };
    /// When the ambient session's open block started — see [`set_block_start`].
    static BLOCK_START: std::cell::Cell<i64> = const { std::cell::Cell::new(NO_INSTANT) };
}

/// Which of the three clocks a builtin reads — the module doc says which spelling
/// reads which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    /// The transaction's start.
    Transaction,
    /// The statement's start.
    Statement,
    /// The wall clock, read at the call.
    Wall,
}

/// A statement starts: whatever it runs reads a statement start of its own.
///
/// A host calls it as a call or a client message reaches it, ahead of whatever the
/// message runs — the embed facade at each public entry, the native server per
/// protocol message that executes. A message carrying several statements therefore
/// runs them under one instant, as 18.6's simple-query message does.
///
/// Natively the instant is read here; on wasm at its first use (the module doc). The
/// callers are `pglet::Database`'s entry guard, which every public entry of the facade
/// passes — the wasm bindings run on it — and the native server's
/// `execute::execute_query_core` and `copy::dispatch_copy`.
pub fn start_statement() {
    STATEMENT_START.set(NO_INSTANT);
    #[cfg(not(target_arch = "wasm32"))]
    statement_start();
}

/// The ambient session's open block started at `start` — [`NO_INSTANT`] with no
/// block open, or one that kept none. `SessionSettings::install_ambient` says so for
/// each statement, beside the session it installs.
pub(crate) fn set_block_start(start: i64) {
    BLOCK_START.set(start);
}

/// When the running statement started: the wall clock at the first call since
/// [`start_statement`] reset it — natively its own — and that same instant at every
/// later one.
///
/// [`NO_INSTANT`] where the clock cannot be read; the next call reads it again.
pub(crate) fn statement_start() -> i64 {
    let mut start = STATEMENT_START.get();
    if start == NO_INSTANT {
        start = wall_clock().map_or(NO_INSTANT, |at| PgTimestamp::Finite(at).to_pg_micros());
        STATEMENT_START.set(start);
    }
    start
}

/// Read `clock`.
///
/// # Errors
///
/// See [`wall_clock`] — raised for [`Clock::Wall`], and for the other two where the
/// statement's start could not be read.
fn read_clock(clock: Clock) -> Result<time::OffsetDateTime> {
    let block = BLOCK_START.get();
    let fixed = if clock == Clock::Transaction && block != NO_INSTANT {
        block
    } else {
        // The wall clock takes this too, so the statement's start is not later than a
        // wall-clock read the statement makes while the clock is not set back between
        // the two (the module doc).
        statement_start()
    };
    match PgTimestamp::from_pg_micros(fixed) {
        Some(PgTimestamp::Finite(at)) if clock != Clock::Wall => Ok(at),
        _ => wall_clock(),
    }
}

/// The current instant in UTC — the wall clock, which `clock_timestamp()` answers and
/// [`statement_start`] reads once a statement. Native reads the OS clock; wasm sources
/// it from the JS host.
///
/// # Errors
///
/// `22008 timestamp out of range` when the clock reads outside the date and timestamp
/// domain (`pglet_core::types::check_timestamp_domain`) — a host clock set before year
/// 1 — and, on wasm, a `22000` when the JS clock yields a value `OffsetDateTime`
/// cannot hold at all, or when `Date.now` throws, or answers `NaN` or no number.
#[cfg(not(target_arch = "wasm32"))]
pub fn wall_clock() -> Result<time::OffsetDateTime> {
    // `OffsetDateTime::now_utc()` reads `clock_gettime`, which carries nanoseconds
    // Postgres's own `gettimeofday`-based clock never sees — and which every pglet
    // encoder divides away. Truncating here is what keeps `Value::Timestamp`
    // microsecond-exact on this leg; without it two reads inside one microsecond
    // compare unequal while rendering one string (see `truncate_timestamp_to_micros`).
    pglet_core::types::check_timestamp_domain(pglet_core::types::truncate_timestamp_to_micros(
        time::OffsetDateTime::now_utc(),
    ))
}

/// The wasm leg of [`wall_clock`]: `Date.now()` as it is at this read, stepped a
/// microsecond past the last read inside that millisecond, up to its tie point
/// (`step_host_clock`).
///
/// # Errors
///
/// See the native leg's.
#[cfg(target_arch = "wasm32")]
pub fn wall_clock() -> Result<time::OffsetDateTime> {
    use std::cell::Cell;
    // wasm is single-threaded, so a thread-local is the module's one clock: every
    // database of the module reads it, and two of them reading in one millisecond are
    // ordered as two statements of one are.
    thread_local! {
        static LAST_READ: Cell<i128> = const { Cell::new(i128::MIN) };
    }
    // `Date.now()` is integer milliseconds (exact in f64); go through integer
    // microseconds rather than scaling the float.
    // Through `crate::host`, which catches: `Date.now` is a global a page can replace.
    #[allow(clippy::cast_possible_truncation)]
    let millis = crate::host::date_now()? as i64;
    let micros = step_host_clock(LAST_READ.get(), millis);
    LAST_READ.set(micros);
    time::OffsetDateTime::from_unix_timestamp_nanos(micros * 1_000)
        .map_err(|e| crate::Error::Execution(format!("now(): {e}")))
        .and_then(pglet_core::types::check_timestamp_domain)
}

/// How far past the start of the JS clock's millisecond [`step_host_clock`] steps before
/// its reads tie.
///
/// The last microsecond that a column of any sub-second precision from 3 up rounds
/// back into the millisecond: `timestamptz(3)` rounds half away from the value's zero
/// (`pglet_core::types::round_micros_to_precision`), so one microsecond more is the next
/// millisecond for an instant after 2000-01-01, and a value stamped there under a
/// pinned clock would read back a millisecond past the pin.
#[cfg(any(target_arch = "wasm32", test))]
const HOST_CLOCK_TIE_MICROS: i128 = 499;

/// The instant a read of the JS clock answers, in microseconds since the Unix epoch:
/// `millis` is what `Date.now()` answered at this read, `last` the instant the read
/// before it answered.
///
/// **The answer is in the first half of the millisecond the JS clock reports** —
/// [`HOST_CLOCK_TIE_MICROS`] past its start at most — so a clock that moves, forward or
/// back, is answered as it reads: a test runner's fake timer pinned to a date is that
/// date, to the millisecond, at any read count. The first read of a millisecond answers
/// its start.
///
/// `Date.now()` is only millisecond-precision, so rapid successive reads would tie —
/// making `ORDER BY created_at` (e.g. "keep newest N") ambiguous between rows two
/// statements stamped, where the native clock keeps them distinct. So a read that finds
/// the read before it in the same millisecond is put one **microsecond** past it, up to
/// the tie point, which that read and every read after it in the millisecond answer.
/// Those reads tie — as two reads of 18.6's clock inside one microsecond do — where a
/// further step would round, in a precision-3 column, to a millisecond the JS clock has
/// not reached.
///
/// The step is a microsecond and not a nanosecond because a nanosecond is a
/// distinction no encoder can carry: it stepped values apart for `values_equal`
/// while the text, binary and index-key encoders divided the difference away, so
/// three stamps rendered one string and `SELECT DISTINCT` answered three —
/// and the ordering this step exists to provide was itself lost at any
/// index-backed `ORDER BY`, whose key is microseconds. A microsecond step keeps
/// the order *and* makes it observable.
#[cfg(any(target_arch = "wasm32", test))]
fn step_host_clock(last: i128, millis: i64) -> i128 {
    let host = i128::from(millis) * 1_000;
    let end = host + HOST_CLOCK_TIE_MICROS;
    if (host..=end).contains(&last) {
        end.min(last + 1)
    } else {
        host
    }
}

/// A timestamp spelling's value: `clock`, as a timestamp.
///
/// # Errors
///
/// See [`read_clock`].
pub fn timestamp(clock: Clock) -> Result<Value> {
    read_clock(clock).map(Value::timestamp)
}

/// `CURRENT_DATE` — the UTC date of the transaction's start, matching Postgres at
/// `TimeZone=UTC`.
///
/// # Errors
///
/// See [`read_clock`].
pub fn current_date() -> Result<Value> {
    read_clock(Clock::Transaction).map(|at| Value::date(at.date()))
}

/// `CURRENT_TIME` / `LOCALTIME` — the UTC time of day of the transaction's start, as
/// a plain `time`.
///
/// Microseconds since midnight, matching Postgres at `TimeZone=UTC`. Postgres's
/// `CURRENT_TIME` is `timetz`; pglet is UTC-only and models both as a plain `time`
/// (documented divergence).
///
/// # Errors
///
/// See [`read_clock`].
pub fn current_time() -> Result<Value> {
    read_clock(Clock::Transaction).map(|at| {
        let micros = (i64::from(at.hour()) * 60 + i64::from(at.minute())) * 60_000_000
            + i64::from(at.second()) * 1_000_000
            + i64::from(at.microsecond());
        Value::Time(micros)
    })
}

/// `gen_random_uuid()` / `uuid_generate_v4()` — a random RFC 4122 v4 UUID.
///
/// # Errors
///
/// Native: errors if the OS RNG fails. Wasm: errors if `Math.random` throws, or answers
/// `NaN` or no number (`crate::host`).
#[cfg(not(target_arch = "wasm32"))]
pub fn random_uuid() -> Result<Value> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| Error::Execution(format!("gen_random_uuid: {e}")))?;
    Ok(Value::Uuid(uuid_v4_bits(bytes)))
}

#[cfg(target_arch = "wasm32")]
pub fn random_uuid() -> Result<Value> {
    // `Math.random()` is not crypto-grade, but the wasm build is a best-effort
    // embedded/test target where collision-free uniqueness is what matters. Each draw
    // gives the top 48 bits of its mantissa, so a uuid is three host calls, the first
    // draw's high 16 bits shifted out; V8's and JavaScriptCore's draws vary in all 52.
    // A page's own `Math.random` answering outside `[0, 1)` cannot trap the cast: below
    // 0 or from `2^16` up it saturates, and in between a draw's bits past the 48th land
    // in the previous draw's place. The version and variant bits are stamped over either.
    let mut bits = 0u128;
    for _ in 0..3 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let draw = (crate::host::math_random()? * 281_474_976_710_656.0) as u64; // 2^48
        bits = (bits << 48) | u128::from(draw);
    }
    Ok(Value::Uuid(uuid_v4_bits(bits.to_le_bytes())))
}

/// `version()` — the build banner a client's version sniff reads.
///
/// **Fabricated, and deliberately not 18.6's own string**: a real one names a
/// PostgreSQL build pglet is not (`PostgreSQL 18.6 (Debian 18.6-1.pgdg13+2) on
/// x86_64-pc-linux-gnu, compiled by gcc …`). It keeps the two things a consumer
/// parses — the `PostgreSQL <release>` prefix every version sniff reads, and the
/// ` on <platform>` clause the looser ones split on — and says `pglet` where the
/// packaging would be.
///
/// The release is [`crate::catalog::SERVER_VERSION`], the same constant
/// `SHOW server_version` reports, so the two cannot drift; the platform is the
/// build target, which differs between the native and wasm legs, so no fixture can
/// compare this string byte-for-byte against 18.6 (see the `version()` fixtures,
/// which assert the prefix).
#[must_use]
pub fn version() -> Value {
    // `wasm32-unknown-unknown` has no OS, so its `consts::OS` is empty.
    let os = match std::env::consts::OS {
        "" => "unknown",
        os => os,
    };
    Value::Text(format!(
        "PostgreSQL {} (pglet) on {}-{os}, {}-bit",
        crate::catalog::SERVER_VERSION,
        std::env::consts::ARCH,
        usize::BITS,
    ))
}

/// `random()` — a `double precision` in `[0, 1)`, from the same source
/// [`random_uuid`] reads: the OS RNG natively, `Math.random` in the wasm build.
///
/// Each answer is a multiple of `2^-52` below 1, as 18.6's `pg_prng_double` draws one
/// (52 random bits over `2^52`).
///
/// # Errors
///
/// [`random_uuid`]'s: the OS RNG failing natively; `Math.random` throwing, or answering
/// `NaN` or no number, in the wasm build.
#[cfg(not(target_arch = "wasm32"))]
pub fn random() -> Result<Value> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(|e| Error::Execution(format!("random: {e}")))?;
    Ok(Value::Float8(unit_interval(u64::from_le_bytes(bytes))))
}

#[cfg(target_arch = "wasm32")]
pub fn random() -> Result<Value> {
    Ok(Value::Float8(unit_interval_from_host(
        crate::host::math_random()?,
    )))
}

/// The `[0, 1)` value of 52 of `bits`' bits: the high 52, over `2^52`.
#[cfg(any(not(target_arch = "wasm32"), test))]
fn unit_interval(bits: u64) -> f64 {
    // a 52-bit integer is exact in an `f64`
    #[allow(clippy::cast_precision_loss)]
    let mantissa = (bits >> 12) as f64;
    mantissa / 4_503_599_627_370_496.0 // 2^52
}

/// A host draw brought into `[0, 1)` whatever the host answered: its 52 bits below the
/// point, so a page's own `Math.random` answering 1, a negative or a large number still
/// yields a value in range — the cast saturates below 0 and above `2^64`, and the mask
/// keeps the fraction's bits.
#[cfg(any(target_arch = "wasm32", test))]
fn unit_interval_from_host(draw: f64) -> f64 {
    const MASK: u64 = (1 << 52) - 1;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let fraction = ((draw * 4_503_599_627_370_496.0) as u64 & MASK) as f64; // 2^52
    fraction / 4_503_599_627_370_496.0
}

/// Stamp the RFC 4122 v4 version (4) and variant (10xx) bits onto random bytes.
const fn uuid_v4_bits(mut bytes: [u8; 16]) -> [u8; 16] {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `random()`'s range at its edges: every bit set is the largest value below 1 and
    /// none set is 0, natively; a host draw at or past 1, negative, or huge stays in
    /// `[0, 1)`, and an ordinary draw keeps its value.
    #[test]
    fn random_stays_in_the_unit_interval_at_every_edge() {
        assert_eq!(unit_interval(0).to_bits(), 0.0f64.to_bits());
        let top = unit_interval(u64::MAX);
        assert!(top < 1.0 && top > 0.999_999_999, "{top}");
        for draw in [
            0.0,
            0.5,
            0.999_999_999_999,
            1.0,
            1.5,
            -0.25,
            1e300,
            f64::INFINITY,
        ] {
            let v = unit_interval_from_host(draw);
            assert!((0.0..1.0).contains(&v), "host draw {draw} answered {v}");
        }
        assert!((unit_interval_from_host(0.5) - 0.5).abs() < f64::EPSILON);
    }

    /// Each call is a fresh draw: a thousand native calls are not all one value, and
    /// each is a `float8` in range.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn random_draws_afresh_at_each_call() {
        let draws: Vec<f64> = (0..1_000)
            .map(|_| match random().expect("the OS RNG answers") {
                Value::Float8(f) => f,
                other => panic!("random() answered {other:?}"),
            })
            .collect();
        assert!(draws.iter().all(|f| (0.0..1.0).contains(f)), "{draws:?}");
        let first = draws[0].to_bits();
        assert!(
            draws.iter().any(|f| f.to_bits() != first),
            "every draw equal"
        );
    }

    /// The native leg's half of the [`Value::Timestamp`] microsecond invariant.
    ///
    /// No differential fixture can reach this — `now()` is nondeterministic, so the
    /// harness has nothing to compare — and no *single* call proves it either: an
    /// untruncated `clock_gettime` read lands on a microsecond boundary about once
    /// in a thousand. The loop is the guard; drop the truncation in `wall_clock` and it
    /// reddens on the first read that carries nanoseconds.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_wall_clock_is_microsecond_exact() {
        for _ in 0..1_000 {
            let ts = wall_clock().expect("the clock reads");
            assert_eq!(
                ts.unix_timestamp_nanos() % 1_000,
                0,
                "the clock carried nanoseconds no encoder can render: {ts}"
            );
        }
    }

    /// Drive [`step_host_clock`] over `reads` of the JS clock, as the wasm `wall_clock`
    /// does: each read's answer is the next one's `last`.
    fn step_through(reads: &[i64]) -> Vec<i128> {
        let mut last = i128::MIN;
        reads
            .iter()
            .map(|&millis| {
                last = step_host_clock(last, millis);
                last
            })
            .collect()
    }

    /// Reads in one millisecond of the JS clock are a microsecond apart, in order, and
    /// the first of them is the millisecond itself.
    #[test]
    fn a_host_clock_that_has_not_moved_steps_a_microsecond_a_read() {
        assert_eq!(
            step_through(&[7, 7, 7, 8, 8]),
            [7_000, 7_001, 7_002, 8_000, 8_001]
        );
    }

    /// A JS clock that moves is answered as it reads, back or forward, at the read that
    /// first sees it: the millisecond's start, whatever was answered under the last one.
    #[test]
    fn a_host_clock_that_moves_is_followed_to_the_millisecond() {
        assert_eq!(
            step_through(&[9_000, 9_000, 5, 5, 5, 9_000, 9_000, 9_001, -3, -3]),
            [
                9_000_000, 9_000_001, 5_000, 5_001, 5_002, 9_000_000, 9_000_001, 9_001_000, -3_000,
                -2_999
            ]
        );
    }

    /// The step stops at the tie point: the reads of one millisecond up to it are
    /// distinct and in order, the read that reaches it answers `+499`, and each read
    /// past it answers that microsecond again — not a later one, and not an earlier one.
    #[test]
    fn the_step_ties_at_the_tie_point_of_the_host_millisecond() {
        let answered = step_through(&vec![7; 2_500]);
        assert_eq!(answered[..3], [7_000, 7_001, 7_002]);
        assert_eq!(answered[497..502], [7_497, 7_498, 7_499, 7_499, 7_499]);
        assert!(answered[..500].windows(2).all(|pair| pair[0] < pair[1]));
        assert!(answered[500..].iter().all(|&at| at == 7_499));
    }

    /// After any number of reads in one millisecond, the next millisecond is answered at
    /// its own start — later than every one of them — and so is the same millisecond
    /// read again after another.
    #[test]
    fn a_host_clock_moving_on_from_a_spent_millisecond_answers_its_own() {
        let mut reads = vec![7; 1_502];
        reads.extend([8, 8, 10, 7, 7]);
        let answered = step_through(&reads);
        assert_eq!(
            answered[1_501..],
            [7_499, 8_000, 8_001, 10_000, 7_000, 7_001]
        );
        assert!(answered[..1_505].windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// Every instant the step answers, rounded as a column of sub-second precision 3
    /// through 6 rounds it, is inside the millisecond the JS clock reported — and at
    /// precision 3 is that millisecond — on both sides of 2000-01-01, where the rounding
    /// changes direction, and before 1970.
    #[test]
    fn a_stepped_instant_rounds_into_the_host_millisecond_at_precision_three_and_finer() {
        use pglet_core::types::round_micros_to_precision;
        // 2000-01-01 in microseconds since the Unix epoch: a timestamp rounds about it.
        const PG_EPOCH: i128 = 946_684_800_000_000;
        // 2024-01-01, 1985-11-05 and 1960-06-30, in milliseconds since the Unix epoch.
        for millis in [1_704_067_200_000_i64, 500_000_000_000, -300_000_000_000] {
            let host = i128::from(millis) * 1_000 - PG_EPOCH;
            for answered in step_through(&[millis; 1_200]) {
                let pg_micros = i64::try_from(answered - PG_EPOCH).expect("in range");
                for precision in 3..=6 {
                    let rounded = round_micros_to_precision(pg_micros, precision)
                        .map(i128::from)
                        .expect("rounds");
                    assert!(
                        (host..host + 1_000).contains(&rounded),
                        "{answered} at ({precision}) left the millisecond of {millis}"
                    );
                }
                assert_eq!(
                    round_micros_to_precision(pg_micros, 3).map(i128::from),
                    Some(host)
                );
            }
        }
    }

    /// Natively a statement's start is read where the host marks its arrival, not
    /// where the statement first reads it.
    ///
    /// The wall clock is made to move between the mark and the statement's first read,
    /// so a start read at the first use is later than `marked_by` and reddens here in
    /// an unoptimized build too — which `pglet`'s
    /// `a_statements_start_is_before_its_wall_clock_reads` does only under
    /// `cargo test --release`.
    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn the_native_statement_start_is_read_at_the_mark() {
        start_statement();
        let marked_by = wall_clock().expect("the clock reads");
        while wall_clock().expect("the clock reads") <= marked_by {}
        let start = read_clock(Clock::Statement).expect("the statement's start reads");
        assert!(
            start <= marked_by,
            "the statement's start {start} was read after its arrival was marked ({marked_by})"
        );
    }
}
