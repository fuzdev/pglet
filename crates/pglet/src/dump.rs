//! Native SQL dump emitter: a pglet catalog + its data → a single portable SQL
//! script whose target is to recreate the database on **both** stock Postgres and
//! pglet — the dump invariant `docs/sequences.md` states, which `EDGES.md` records
//! databases falling short of.
//!
//! The shape follows `pg_dump`'s "schema, then data, then constraints" ordering so
//! a self-referential or cross-table foreign key never blocks the bulk load: every
//! `CREATE SEQUENCE` / `CREATE TYPE` / `CREATE TABLE` is emitted first, then one
//! `COPY … FROM stdin` block per table (reusing the same [`execute_copy_to`] text
//! encoder the wire `COPY` path uses), then the `PRIMARY KEY` / `UNIQUE` / index /
//! `CHECK` definitions, and finally every `FOREIGN KEY` — so all referenced keys
//! exist before any FK is added.
//!
//! A **plain** sequence takes **three** statements rather than one, because its start,
//! its owner, and its counter are three separate facts landing at three different
//! points in that ordering: `CREATE SEQUENCE … START WITH` in the schema block, an
//! `ALTER SEQUENCE … OWNED BY` after the tables (it names one), and a `setval` after
//! the data (a `COPY` writes explicit ids without advancing the counter). pg_dump
//! splits it the same way, for the same reasons.
//!
//! An **identity** sequence takes **two**, at two points, and shares none of the
//! first two statements: one `ALTER TABLE … ALTER COLUMN … ADD GENERATED … AS
//! IDENTITY (SEQUENCE NAME …)` after its table, plus the same trailing `setval`. That
//! single statement creates the sequence, records the ownership edge, and sets the
//! column `NOT NULL`, so emitting a `CREATE SEQUENCE` beside it is `42P07` on the
//! restore and an `ALTER SEQUENCE … OWNED BY` beside it is `0A000` — suppressing both
//! is required, not tidiness. Rendering an identity column as an ordinary
//! `nextval`-defaulted one instead loses the identity fact entirely: `GENERATED
//! ALWAYS` stops rejecting user-supplied ids, and the three refusals that keep an
//! identity column tied to its sequence (`ALTER COLUMN … DROP DEFAULT`,
//! `ALTER SEQUENCE … OWNED BY`, `DROP SEQUENCE`) all stop firing on the restored copy.
//!
//! # No `CREATE TABLE` names a sequence
//!
//! Every `DEFAULT nextval(…)` is **hoisted** out of the `CREATE TABLE` into an
//! `ALTER TABLE … ALTER COLUMN … SET DEFAULT` emitted after the whole schema block
//! (pg_dump's own idiom for the case it must break a dependency cycle for). Once an
//! identity sequence is created *after* its table rather than in the sequence block,
//! the guarantee that made an inline default safe — every sequence exists before
//! every table — is gone, and two ordinary catalogs walk straight through the hole: a
//! sibling column defaulting from the same table's identity sequence, and a column of
//! a table earlier in name order defaulting from another table's. Hoisting *every*
//! such default removes the ordering requirement rather than patching its two known
//! instances, so nothing in the schema block depends on where a sequence is created.
//! Non-sequence defaults (literals, `now()`, `gen_random_uuid()`) stay inline.
//!
//! # Correct-or-loud
//!
//! The emitter never guesses. A `DEFAULT` it can't reproduce faithfully, an
//! enum/identifier that isn't expressible, a `CHECK` / index-predicate /
//! index-key expression outside the rendered subset, an identity column's sequence
//! that shares its name with an enum, or a table that does, each return an `Err` rather
//! than emit SQL that would silently diverge on reload. A column *type* is no
//! longer among them: `type_oid_to_sql_name` spells every scalar pglet has
//! (`interval` included) and [`type_oid_sql`] spells an array from its element, so
//! `pglet_sql::deparse`'s `unsupported_type` has two live call sites that cannot
//! currently fire.
//!
//! Output is **deterministic**: every catalog map (tables, enums, sequences,
//! indexes) is walked in name-sorted order, columns in catalog (position) order,
//! so the same database always dumps to the same bytes.

use std::fmt::Write as _;

use pglet_core::Storage;
use pglet_sql::catalog::{
    Column, ColumnDefault, ConstraintEntryKind, ConstraintKind, ForeignKey, IdentityGeneration,
    Index, IndexKey, Schema, Sequence, SequenceOwner, Table, constraint_def,
};
use pglet_sql::deparse::{
    RenderMode, bare_type_ident, quote_name_literal, quote_string_literal, render_expr,
    type_oid_sql,
};
use pglet_sql::executor::{CopyParams, default_sequence_refs, execute_copy_to};
use pglet_sql::lexer::quote_ident;
use pglet_sql::planner::CopyFormat;
use pglet_sql::types::{JsonbValue, TypeOid, Value};
use pglet_sql::{Error, Result};

/// The COPY null marker the dump emits (and the inverse loader detects): the
/// Postgres default `\N`.
const NULL_MARKER: &str = "\\N";

/// Emit a portable SQL dump of `schema` + the data in `storage`.
///
/// The returned script's target is to load into stock Postgres and back into pglet
/// (via `psql`'s inline `COPY … FROM stdin`, or by splitting statements and driving
/// `copy_in`) — the dump invariant `docs/sequences.md` states, which `EDGES.md`
/// records databases falling short of.
/// It is a pure read — neither `schema` nor `storage` is mutated.
///
/// # Errors
///
/// Returns an error if any catalog element can't be rendered to round-trippable
/// SQL (an unsupported column type, an unreproducible `DEFAULT`, a non-expressible
/// identifier, a `CHECK` / index expression outside the rendered subset, or an identity
/// sequence or a table named like an enum — [`refuse_identity_sequence_named_like_an_enum`],
/// [`refuse_table_named_like_an_enum`]), a
/// storage error from scanning a table's rows, or `Error::InvalidEncoding` (`22021`)
/// when a table's `COPY` text holds a NUL or a byte that is not valid UTF-8 — or when
/// a stored **jsonb** string holds one, which that text hides behind jsonb's escape
/// ([`refuse_unreadable_values`]); and, behind any jsonb refusal in the same table,
/// `Error::TemporalOutOfRange` (`22008`) for a stored `date` or `timestamp` outside the
/// date domain, or `Error::Execution` (`22000`) for a stored `timestamp` at a non-UTC
/// offset ([`temporal_restore_failure`]).
pub fn dump_sql<S: Storage>(schema: &Schema, storage: &S) -> Result<String> {
    let mut blocks: Vec<String> = Vec::new();

    // The sequences an identity column's `ADD GENERATED` creates for itself. Two
    // blocks read this **set**, to suppress the statements such a sequence must not
    // get (1's `CREATE SEQUENCE`, 3b's `ALTER SEQUENCE … OWNED BY`); two more ask
    // `identity_spec` per column, which is what the set is built from (3a emits the
    // `ADD GENERATED`, 3c suppresses the column's `nextval` default). Four blocks over
    // one derivation, so no two of them can disagree about which statement creates a
    // given sequence. The identity refusal below reads it as well.
    let identity_sequences = identity_sequences(schema);
    refuse_identity_sequence_named_like_an_enum(schema, &identity_sequences)?;
    refuse_table_named_like_an_enum(schema)?;

    // 1. Sequences, ahead of the types — a sequence and an enum may share a name
    //    ([`emit_create_sequence`]'s order note). An identity column's is **not**
    //    created here: its `ADD GENERATED` below creates it (and a `CREATE SEQUENCE`
    //    for the same name makes that statement's declared `SEQUENCE NAME` a `42P07`).
    for name in sorted_keys(schema.sequences.keys()) {
        if identity_sequences.contains(name) {
            continue;
        }
        blocks.push(emit_create_sequence(name, &schema.sequences[name]));
    }

    // 2. User ENUM types (must precede any column that references one).
    for name in sorted_keys(schema.enums.keys()) {
        let members = &schema.enums[name].labels;
        blocks.push(emit_create_type(name, members)?);
    }

    // 3. Tables (no constraints inline except NOT NULL/DEFAULT — PK/UNIQUE/FK/CHECK
    //    are added after the data). A `DEFAULT nextval(…)` is not inline either; see
    //    block 3c.
    let tables = sorted_tables(schema);
    for table in &tables {
        blocks.push(emit_create_table(table)?);
    }

    // 3a. Identity columns, **after** their table (the statement names one) and ahead
    //     of every `nextval` default (a sibling column may draw from the sequence this
    //     creates). Each creates its own sequence, owns it, and re-asserts NOT NULL.
    for table in &tables {
        for col in &table.columns {
            if let Some(spec) = identity_spec(schema, col) {
                blocks.push(emit_add_generated(table, col, &spec));
            }
        }
    }

    // 3b. Sequence ownership, **after** the tables and never in the sequence block —
    //     `OWNED BY` names a table, which does not exist yet up there. Same ordering
    //     constraint pg_dump has, and the reason a sequence needs two statements once
    //     its start and its owner are separate facts. An identity sequence is skipped:
    //     its `ADD GENERATED` already recorded the edge, and re-stating it is `0A000`
    //     ("cannot change ownership of identity sequence") on both engines.
    for name in sorted_keys(schema.sequences.keys()) {
        if identity_sequences.contains(name) {
            continue;
        }
        if let Some(owner) = &schema.sequences[name].owner {
            blocks.extend(emit_sequence_owned_by(schema, name, owner));
        }
    }

    // 3c. Every `DEFAULT nextval(…)`, hoisted out of its `CREATE TABLE` to here — see
    //     the module doc. Last in the schema block, so every sequence exists whether
    //     it was created by a `CREATE SEQUENCE` above or by an `ADD GENERATED`.
    for table in &tables {
        for col in &table.columns {
            blocks.extend(emit_nextval_default(schema, table, col)?);
        }
    }

    // 4. Data — one `COPY … FROM stdin` block per table, in name order.
    for table in &tables {
        blocks.push(emit_copy(schema, storage, table)?);
    }

    // 5. PK / UNIQUE / index / CHECK per table (all referenced keys before any FK).
    for table in &tables {
        emit_table_constraints(&mut blocks, schema, table)?;
    }

    // 6. Foreign keys, so every referenced PK/UNIQUE already exists.
    for table in &tables {
        let mut fks: Vec<&ForeignKey> = table.foreign_keys.iter().collect();
        fks.sort_by(|a, b| a.name.cmp(&b.name));
        for fk in fks {
            blocks.push(emit_foreign_key(table, fk)?);
        }
    }

    // 7. Sequence positions last, after the data — a `COPY` carries explicit id
    //    values and does not advance the sequence, so the counter has to be restored
    //    separately. Same place pg_dump puts it.
    for name in sorted_keys(schema.sequences.keys()) {
        blocks.push(emit_setval(name, schema.sequences[name].last_value));
    }

    let mut out = blocks.join("\n\n");
    out.push('\n');
    Ok(out)
}

// =============================================================================
// Schema objects
// =============================================================================

/// `CREATE TYPE <name> AS ENUM ('a', 'b', …);`
fn emit_create_type(name: &str, members: &[String]) -> Result<String> {
    let labels: Vec<String> = members.iter().map(|m| quote_string_literal(m)).collect();
    Ok(format!(
        "CREATE TYPE {} AS ENUM ({});",
        bare_type_ident(name)?,
        labels.join(", ")
    ))
}

/// `CREATE SEQUENCE <name> [MINVALUE <m>] [START WITH <n>];` — the sequence's
/// **declared start**, which is a distinct fact from its counter.
///
/// **Emitted ahead of every `CREATE TYPE`.** A sequence and an enum may share a name
/// on both engines, but only in that order: `CREATE TYPE` over a sequence's name is
/// accepted (a sequence has no row type), while `CREATE SEQUENCE` over an enum's is
/// `42710` (`heap_create_with_catalog` asks the type namespace of every relation it
/// creates). A sequence's creating statement names no type, so nothing it needs is
/// moved later by going first. An identity column's sequence cannot go first — its
/// `ADD GENERATED` names a table, which may name the enum — and
/// [`refuse_identity_sequence_named_like_an_enum`] answers that pair instead.
///
/// The old emitter folded the two together (`START WITH <counter + 1>`), which
/// restored the right resume point and lost the start; a restored database then
/// answered a bare `ALTER SEQUENCE … RESTART` differently from the one it came
/// from. The counter travels separately, in [`emit_setval`]. `START WITH 1` is the
/// default on both engines, so it is normally omitted.
///
/// **The `MINVALUE` is what keeps a non-positive sequence loadable.** pglet models
/// no `MINVALUE`/`INCREMENT`, so it accepts `CREATE SEQUENCE s START WITH 0`,
/// `ALTER SEQUENCE s RESTART WITH 0` and `setval(s, 0)` where stock Postgres raises
/// `22023`/`22003` — the default minimum for an ascending sequence is `1`, and only
/// an explicit `MINVALUE n` moves it (`NO MINVALUE` and absence both mean `1`,
/// measured on 18.6). That acceptance gap is filed, not closed here; but a catalog
/// that *has* reached a non-positive value must still dump to a script both engines
/// reload, so the emitter states the floor the values need. Emitting it forces the
/// explicit `START WITH` too, even for the default `1`: given a `MINVALUE`, Postgres
/// seeds an omitted `seqstart` from it (`init_params`), so `CREATE SEQUENCE s
/// MINVALUE -6;` would restore with `start = -6` rather than `1`.
fn emit_create_sequence(name: &str, sequence: &Sequence) -> String {
    let ident = quote_ident(name);
    let floor = sequence_floor(sequence);
    if floor < 1 {
        format!(
            "CREATE SEQUENCE {ident} MINVALUE {floor} START WITH {};",
            sequence.start
        )
    } else if sequence.start == 1 {
        format!("CREATE SEQUENCE {ident};")
    } else {
        format!("CREATE SEQUENCE {ident} START WITH {};", sequence.start)
    }
}

/// The lowest value a restore of `sequence` has to accept: its declared start, and
/// the resume point [`emit_setval`] will hand back.
///
/// Below `1` this is the explicit `MINVALUE` the restore needs, because pglet models
/// no minimum and stock Postgres defaults an ascending sequence's to `1`. Shared by
/// both renderings of a sequence's creation — [`emit_create_sequence`] and the
/// identity option list [`emit_add_generated`] builds — since the reasoning is about
/// the *values*, not about which statement declares them: an identity column whose
/// start or counter went below `1` is reachable through pglet's own accepted DDL
/// (`GENERATED ALWAYS AS IDENTITY (START WITH 0)`, `setval(<its sequence>, -5)`) and
/// stock Postgres refuses `START value (0) cannot be less than MINVALUE (1)` without
/// the floor stated. `MINVALUE` is parsed-and-ignored inside an identity option list
/// on pglet, so it is inert on the self-restore and load-bearing on the Postgres one.
const fn sequence_floor(sequence: &Sequence) -> i64 {
    let resume = sequence.last_value.saturating_add(1);
    if sequence.start < resume {
        sequence.start
    } else {
        resume
    }
}

/// `ALTER SEQUENCE <name> OWNED BY <table>.<column>;` — or nothing, when the owner
/// no longer resolves.
///
/// The skip is defense in depth against a **dangling** owner — an owner naming a
/// relation or column the catalog does not have, which renders a statement stock
/// Postgres rejects (`relation "t" does not exist` / `column "a" of relation "u" does
/// not exist`) and, for the column routes, **pglet's own restore rejects too**.
/// Recording a persisted cross-object reference is what turned each pre-existing leak
/// into a rendering bug; the reference was harmless while nothing pointed at it.
///
/// **Every route that used to reach the state is closed**, and each was closed by a
/// separate repair: `DROP TABLE` / `ALTER TABLE … DROP COLUMN` take an owned sequence
/// with them (`executor::dependencies`' AUTO edge), and an `ADD COLUMN … SERIAL` that
/// fails after its sequence would have been registered now registers nothing, because
/// the registration is staged behind the backfill validation (`executor::ddl`'s
/// `staged_sequences`). So this branch has no live SQL seed and its guard —
/// `dump::tests`' `a_dangling_sequence_owner_is_not_emitted` — builds the state on the
/// catalog by hand. That is the shape to keep: a guard whose only seed is a defect
/// stops testing the emitter the moment the defect is fixed.
///
/// Skipping rather than failing loud, deliberately: a sequence whose owner is gone is
/// *semantically* unowned (the edge's only consumers are drops and `TRUNCATE …
/// RESTART IDENTITY`, which have nothing left to act on), and an `Err` here would
/// brick `dump_sql` for the whole database — losing every table's data to preserve an
/// edge that points at nothing, exactly when an export is most wanted. It is not a
/// guess either, which is the bar this module's correct-or-loud rule sets: no fact is
/// invented, one unrepresentable edge is dropped. With every route closed it never
/// fires on a catalog the engine itself wrote; it still stands for one an older store
/// or a future registration site could hand it.
fn emit_sequence_owned_by(schema: &Schema, name: &str, owner: &SequenceOwner) -> Option<String> {
    let owner_exists = schema
        .get_table(&owner.table)
        .is_some_and(|t| t.columns.iter().any(|c| c.name == owner.column));
    owner_exists.then(|| {
        format!(
            "ALTER SEQUENCE {} OWNED BY {}.{};",
            quote_ident(name),
            quote_ident(&owner.table),
            quote_ident(&owner.column)
        )
    })
}

/// `SELECT setval('<name>', <last_value + 1>, false);` — the sequence's counter.
///
/// The `is_called = false` spelling, and that choice is load-bearing rather than
/// stylistic. pglet's counter is the *last issued* value, so the equivalent
/// `is_called = true` form would be `setval(n, last_value, true)` — which stock
/// Postgres **rejects** for a never-advanced sequence, since `last_value` is then
/// `start - 1` and `setval` range-checks against `MINVALUE` (`0` is out of bounds
/// for the default `1..`). The `false` form says "the next `nextval` returns this",
/// which is exactly the fact pglet stores, is in range whenever the counter is,
/// and round-trips pglet byte-exactly (`setval(n, k, false)` sets `last_value =
/// k - 1`). pg_dump uses the same spelling for a sequence that was never called.
///
/// Emitted unconditionally, so the restored counter equals the dumped one whatever
/// the start is.
///
/// The name goes through [`quote_name_literal`], not [`quote_string_literal`]: this is
/// the second of the two literal name positions, and it fails **independently** of the
/// `nextval` one. An identity column emits no `nextval` at all, so its counter travels
/// through this statement alone.
fn emit_setval(name: &str, last_value: i64) -> String {
    format!(
        "SELECT setval({}, {}, false);",
        quote_name_literal(name),
        last_value.saturating_add(1)
    )
}

/// The three catalog facts an identity column's `ADD GENERATED` renders: which
/// generation it is, which sequence feeds it, and that sequence's state.
struct IdentitySpec<'a> {
    generation: IdentityGeneration,
    sequence: &'a str,
    state: &'a Sequence,
}

/// The identity facts `col` carries, or `None` when it is not an identity column the
/// emitter can render.
///
/// **Deliberately a wider predicate than the engine's INTERNAL-edge derivation**
/// (`executor::dependencies::identity_sequence_owner`), and the difference is the
/// point rather than drift: that function additionally requires the sequence's
/// `owner` to name this very column, because its callers ask *"is this sequence this
/// column's implementation?"* — the question a `DROP SEQUENCE` refusal and the
/// `pg_attrdef` exemption turn on. The emitter asks the column-side question, *"is
/// this column an identity column, and what does it draw from?"*, which the column's
/// own `identity` + `NextVal` answer without consulting the sequence at all.
///
/// Where the two answers differ — a catalog whose `identity` flag and `owner` edge
/// came apart, unreachable through the DDL this engine accepts today — the wider one
/// is the safe one. The window a store holding that state can come from is bounded at
/// **both** ends by the same store magic, and the bounds are structural rather than
/// chronological: the state is not *representable* until a sequence becomes a record
/// carrying an `owner` (before that a sequence is a bare counter, and there is no edge
/// to come apart), and that is precisely the bump to the current magic — which is in
/// turn the only magic that still opens, every superseded one failing loud on reopen.
/// So such a store was written by a build carrying today's magic and predating the
/// gates, and no older one can be read at all. The narrow predicate
/// would emit no `ADD GENERATED`, silently restoring an ordinary `nextval` column and
/// losing exactly the guarantee this rendering exists to carry; the wide one restores
/// the identity and re-records the ownership edge, which is the only state the gates
/// now permit anyway (no spelling of the split state survives a restore — `ALTER
/// SEQUENCE <an identity sequence> OWNED BY …` is `0A000` on both engines).
///
/// The one requirement it does not relax is that the sequence **exist**: with no
/// catalog record there is no start and no counter to render, and inventing them is
/// the guess this module refuses. Such a column falls through to the ordinary
/// hoisted-default path, where the dump's own `nextval`-has-a-sequence check names it
/// loudly instead. (A column carrying `identity` with no `NextVal` default at all
/// falls through the same way; no route builds one.)
fn identity_spec<'a>(schema: &'a Schema, col: &'a Column) -> Option<IdentitySpec<'a>> {
    let generation = col.identity?;
    let ColumnDefault::NextVal(sequence) = col.default.as_ref()? else {
        return None;
    };
    Some(IdentitySpec {
        generation,
        sequence,
        state: schema.sequences.get(sequence)?,
    })
}

/// Refuse a catalog in which an identity column's sequence shares its name with an
/// enum — a state both engines build (the identity first, then `CREATE TYPE` over the
/// sequence's name, which has no row type) and that the dump's statement order does not restore.
///
/// The sequence is created by its `ADD GENERATED`, which follows its table, and the
/// types precede every table because a column may declare one; a type of the
/// sequence's name then refuses the `ADD GENERATED` (`42710` on both engines, the
/// type namespace test [`emit_create_sequence`]'s order note describes). Emitting the
/// enum later instead works only while no column or stored expression names it, a
/// reader list this module would have to keep whole. So the export refuses, loudly,
/// rather than emit a script whose `ADD GENERATED` fails — a script that, run by a
/// `psql` without `ON_ERROR_STOP`, exits `0` with the identity lost. 18.6's own
/// `pg_dump` orders this pair the same way, and its script is refused at the same
/// statement (measured).
///
/// The test is exact, as the restore's collision is: the enum is created under its
/// catalog key, and the `ADD GENERATED`'s gate compares that key with the sequence's
/// name byte for byte.
fn refuse_identity_sequence_named_like_an_enum(
    schema: &Schema,
    identity_sequences: &std::collections::BTreeSet<&str>,
) -> Result<()> {
    let shared = identity_sequences
        .iter()
        .find(|name| schema.enums.contains_key(**name));
    shared.map_or(Ok(()), |name| {
        Err(Error::NotImplemented(format!(
            "cannot dump identity sequence {} beside the enum type of the same name: a \
             restore creates the type before the table whose identity column creates the \
             sequence, and the type refuses it",
            quote_ident(name)
        )))
    })
}

/// Refuse a catalog in which a table shares its name with an enum — a pair the type
/// namespace gates refuse to build (a table's row type is in that namespace), and which
/// a store written before those gates can hold and still opens.
///
/// Neither order of the two `CREATE`s restores it: 18.6 answers `42710` to whichever
/// comes second, and so does pglet. The dump creates the types before the
/// tables, so a restore run by a `psql` without `ON_ERROR_STOP` refuses the
/// `CREATE TABLE`, then the table's `COPY`, and exits `0` with the table and its rows
/// lost. So the export refuses, loudly, as it does for a legacy stored expression it
/// cannot render.
///
/// The test is the enum's catalog key against the table's name, byte for byte — which
/// is the reload's own collision, because the script spells both from exactly those: the
/// enum bare through `bare_type_ident` (a key that is not a safe bare identifier fails
/// the dump there), the table through `quote_ident`. The name an enum was written with
/// is not stored — `CREATE TYPE` keeps only its ASCII-folded key — so a table `"Ab"`
/// beside an enum created as `"Ab"` (key `ab`) is a pair that reloads, `CREATE TYPE ab`
/// then `CREATE TABLE "Ab"`, and is not refused.
fn refuse_table_named_like_an_enum(schema: &Schema) -> Result<()> {
    let shared = sorted_keys(schema.tables.keys())
        .into_iter()
        .find(|name| schema.enums.contains_key(*name));
    shared.map_or(Ok(()), |name| {
        Err(Error::NotImplemented(format!(
            "cannot dump table {} beside the enum type of the same name: a table's row \
             type shares the type namespace, so a restore refuses whichever is created second",
            quote_ident(name)
        )))
    })
}

/// Every sequence some identity column's `ADD GENERATED` creates for itself.
///
/// Built from [`identity_spec`] over the same walk that emits those statements, so
/// the suppression of a `CREATE SEQUENCE` / `ALTER SEQUENCE … OWNED BY` and the
/// emission of the statement that replaces them read one derivation. Deriving them
/// separately is how a sequence ends up created twice (`42P07`) or not at all.
fn identity_sequences(schema: &Schema) -> std::collections::BTreeSet<&str> {
    schema
        .tables
        .values()
        .flat_map(|table| table.columns.iter())
        .filter_map(|col| identity_spec(schema, col).map(|spec| spec.sequence))
        .collect()
}

/// `ALTER TABLE <t> ALTER COLUMN <c> ADD GENERATED {ALWAYS | BY DEFAULT} AS IDENTITY
/// (SEQUENCE NAME <s> [MINVALUE <m>] [START WITH <n>]);` — pg_dump's own spelling,
/// and the one statement that carries every identity fact.
///
/// The option list states what the catalog holds and nothing else. `SEQUENCE NAME` is
/// **always** emitted, including when it equals the `<table>_<column>_seq` the restore
/// would mint for itself: a declared name is the user's and is taken verbatim, an
/// omitted one is the engine's and is disambiguated, and the two coincide in the
/// common case — so a dump that leaves it out is right until the first database whose
/// identity sequence is called something else, at which point the trailing `setval`
/// names a sequence the restored catalog does not have and the counter silently
/// restarts at 1. `START WITH` rides along when the start is not the default `1`, and
/// whenever a `MINVALUE` is stated (given a minimum, `init_params` seeds an omitted
/// `seqstart` from it, so the two must travel together). `INCREMENT BY` / `NO
/// MAXVALUE` / `CACHE` / `AS <type>` — which pg_dump spells out and pglet parses and
/// ignores — are not emitted: pglet models none of them, so stating one asserts a fact
/// the catalog does not hold, and Postgres's defaults are pglet's model already.
fn emit_add_generated(table: &Table, col: &Column, spec: &IdentitySpec<'_>) -> String {
    let generation = match spec.generation {
        IdentityGeneration::Always => "ALWAYS",
        IdentityGeneration::ByDefault => "BY DEFAULT",
    };
    let mut options = format!("SEQUENCE NAME {}", quote_ident(spec.sequence));
    let floor = sequence_floor(spec.state);
    if floor < 1 {
        let _ = write!(options, " MINVALUE {floor}");
    }
    if floor < 1 || spec.state.start != 1 {
        let _ = write!(options, " START WITH {}", spec.state.start);
    }
    format!(
        "ALTER TABLE {} ALTER COLUMN {} ADD GENERATED {generation} AS IDENTITY ({options});",
        quote_ident(&table.name),
        quote_ident(&col.name),
    )
}

/// `ALTER TABLE <t> ALTER COLUMN <c> SET DEFAULT nextval('<s>'::regclass);` for a
/// column drawing from a sequence — or nothing for any other column.
///
/// Every `nextval` default is emitted here rather than inline in the `CREATE TABLE`,
/// so no table's creation depends on a sequence existing yet (see the module doc). An
/// **identity** column emits nothing at all: its [`emit_add_generated`] already set
/// the same default, and re-stating it would be a second `pg_attrdef` row for a
/// column Postgres deliberately keeps none for. **Both** engines refuse it with
/// `42601` (`column "…" of relation "…" is an identity column`), so emitting it would
/// fail the restore on either leg rather than diverge quietly.
fn emit_nextval_default(schema: &Schema, table: &Table, col: &Column) -> Result<Option<String>> {
    let Some(default) = col.default.as_ref().filter(|d| default_names_a_sequence(d)) else {
        return Ok(None);
    };
    if identity_spec(schema, col).is_some() {
        return Ok(None);
    }
    // Through the one default renderer, not a second copy of its `nextval(…)`
    // spelling: the dump's own `nextval`-has-a-sequence check reads that spelling, so
    // two sites that could drift would silently take a default out of its scope.
    Ok(Some(format!(
        "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {};",
        quote_ident(&table.name),
        quote_ident(&col.name),
        render_default(default)?,
    )))
}

/// Whether `default` names a sequence anywhere — the predicate that decides which
/// side of the hoist a column's default falls on.
///
/// It is **"references a sequence"**, not "is a bare `nextval`", and the widening is
/// the price of a general expression default: `DEFAULT nextval('s') + 1` names `s`
/// exactly as the bare spelling does, so leaving it inline would put a
/// `CREATE TABLE` ahead of its own `CREATE SEQUENCE` — the loop the hoist exists to
/// break (see the module doc), and a restore stock Postgres refuses.
///
/// Every sequence function counts, matching `executor::dependencies`'
/// `default_sequence_refs`: the two answer the same question about the same
/// persisted shape, one for the dump's ordering and one for the `DROP` edge, and a
/// default one of them saw as sequence-free would either be emitted too early or
/// left holding a dropped sequence.
fn default_names_a_sequence(default: &ColumnDefault) -> bool {
    !default_sequence_refs(default).is_empty()
}

/// `CREATE TABLE <name> (\n  <col> <type> [NOT NULL] [DEFAULT <x>],\n …\n);`
///
/// A default that **names a sequence** is not among the rendered ones — it is hoisted
/// to [`emit_nextval_default`] so that no `CREATE TABLE` names a sequence.
fn emit_create_table(table: &Table) -> Result<String> {
    let mut out = format!("CREATE TABLE {} (\n", quote_ident(&table.name));
    for (i, col) in table.columns.iter().enumerate() {
        let mut line = format!("    {} {}", quote_ident(&col.name), column_type_sql(col)?);
        if !col.nullable {
            line.push_str(" NOT NULL");
        }
        if let Some(default) = &col.default
            && !default_names_a_sequence(default)
        {
            let _ = write!(line, " DEFAULT {}", render_default(default)?);
        }
        if i + 1 < table.columns.len() {
            line.push(',');
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str(");");
    Ok(out)
}

/// The Postgres DDL type spelling for a column: an enum type name, a `<element>[]`
/// array spelling, or the scalar's round-trippable name.
///
/// A modifier is the **element's**, and so is spelled inside the `[]` rather than
/// after it: a `varchar(3)[]` column dumps as `varchar(3)[]`, which is what both
/// engines re-read as an array of `varchar(3)`. Rendering the modifier at the column's
/// width instead would emit a *scalar* `varchar(3)` and reload the array column as a
/// scalar one.
///
/// All three modifiers ride out: a `numeric(p,s)`'s scale rounding, a
/// `varchar(n)`'s truncation and a `timestamp(p)`/`interval(p)`'s sub-second
/// rounding are each **re-applied by the restore**, so dumping any of them bare
/// changes stored values on reload without saying so.
fn column_type_sql(col: &Column) -> Result<String> {
    // An enum column stores as text but reports its enum type name (the bare
    // identifier the type-name parser requires — quoting a type name there fails).
    if let Some(enum_name) = &col.enum_type {
        return bare_type_ident(enum_name);
    }
    let element = col.type_oid.array_element().unwrap_or(col.type_oid);
    let suffix = if col.type_oid.array_element().is_some() {
        "[]"
    } else {
        ""
    };
    // A `NUMERIC(precision, scale)` column round-trips its modifier, so a restore
    // re-applies the same scale rounding (dumping it as bare `numeric` would drop
    // that and silently change stored values).
    if let (TypeOid::Numeric, Some(tm)) = (element, col.numeric_typmod) {
        return Ok(format!("numeric({},{}){suffix}", tm.precision, tm.scale));
    }
    // A `varchar(n)` / `char(n)` column round-trips its length so a restore
    // re-applies the same store-time truncation. (Both spell as `varchar(n)` — pglet
    // maps `char` to the same `TypeOid::Varchar` and doesn't model bpchar padding.)
    if let (TypeOid::Varchar, Some(n)) = (element, col.varchar_length) {
        return Ok(format!("varchar({n}){suffix}"));
    }
    // A `timestamp(p)` / `timestamptz(p)` / `time(p)` / `interval(p)` column
    // round-trips its precision so a restore re-applies the same rounding: dumping it
    // bare drops the rounding silently, which is the shape a `numeric(p,s)` dumped as
    // `numeric` would have. The spelling is the **portable** one (`timestamp(3)`, not
    // `timestamp(3) without time zone`) — stock 18.6 accepts both, and this one
    // re-parses here too.
    if let (
        TypeOid::Timestamp | TypeOid::Timestamptz | TypeOid::Time | TypeOid::Interval,
        Some(p),
    ) = (element, col.temporal_precision)
    {
        return Ok(format!("{}({p}){suffix}", type_oid_sql(element)?));
    }
    type_oid_sql(col.type_oid)
}

/// A `DEFAULT` expression rendered back to SQL.
///
/// The `nextval` spellings differ by arm on purpose. The closed
/// [`ColumnDefault::NextVal`] keeps pg_dump's `nextval('s'::regclass)`, which the
/// column-`DEFAULT` position's token-level classifier reads; a `nextval` **inside**
/// an expression is rendered bare by [`render_expr`], because that one re-parses
/// through the general expression grammar, which refuses the `::regclass` spelling with
/// `0A000 not implemented: nextval() requires a string-literal sequence name`.
fn render_default(default: &ColumnDefault) -> Result<String> {
    Ok(match default {
        ColumnDefault::Literal(v) => render_default_literal(v)?,
        ColumnDefault::Now => "now()".to_string(),
        // Each `now`-family spelling round-trips as itself: `pg_dump` preserves the
        // written form (measured), and re-minting them all as `now()` would silently
        // turn a volatile `clock_timestamp()` default into a stable one across a
        // dump/restore. All five are legal DDL on both engines.
        ColumnDefault::TransactionTimestamp => "transaction_timestamp()".to_string(),
        ColumnDefault::StatementTimestamp => "statement_timestamp()".to_string(),
        ColumnDefault::ClockTimestamp => "clock_timestamp()".to_string(),
        ColumnDefault::CurrentTimestamp => "CURRENT_TIMESTAMP".to_string(),
        ColumnDefault::CurrentDate => "CURRENT_DATE".to_string(),
        ColumnDefault::CurrentTime => "CURRENT_TIME".to_string(),
        ColumnDefault::RandomUuid => "gen_random_uuid()".to_string(),
        // pg_dump's regclass form; both engines parse it and (re)resolve the name —
        // through `quote_name_literal`, so the name inside the literal resolves to
        // the one the identifier positions spell.
        ColumnDefault::NextVal(seq) => {
            format!("nextval({}::regclass)", quote_name_literal(seq))
        }
        // A general expression goes through the **one** `Expr` deparser
        // (`pglet_sql::deparse`), the same one `information_schema.columns`'
        // `column_default` reads — two renderings of one persisted shape is the drift
        // seam this engine keeps paying for, and it would be quiet here: nothing
        // compares a dump's spelling with an introspecting client's. It fails loud on
        // a tree it cannot spell, which is this side's whole reason for having an
        // error channel.
        //
        // **What keeps that error unreachable is the render probe in
        // `executor::ddl`'s `cook_column_default`, not the node walk beside it.**
        // `check_default_expr_nodes` refuses the constructs *18.6* forbids in a
        // `DEFAULT`, which is a different set from the ones this deparser has no
        // spelling for — an interval literal is legal there and is an `Expr::Bound`
        // here — so the cook runs `render_expr` itself, last, as its own
        // over-rejection. That probe used to be skipped whenever the expression's
        // *evaluation* failed with a deferred code, and one statement then reached
        // this arm: `DEFAULT now() + interval '1 day' * (1/0)` was accepted and
        // `pglet_server dump` exited 1 for the **whole database**, every other
        // table's schema and rows lost from the export.
        // `pglet`'s `a_database_with_any_accepted_expression_default_dumps` is the
        // guard from this side.
        ColumnDefault::Expr(expr) => render_expr(expr)?,
    })
}

/// Render a literal `DEFAULT` value. Only the value shapes the DDL parser produces
/// (number / string / boolean / NULL) are reproducible; anything else fails loud
/// rather than emit a possibly-wrong cast.
fn render_default_literal(value: &Value) -> Result<String> {
    Ok(match value {
        // Not reached, and the reason is now upstream of every caller rather than a
        // belt one of them wears: the parser reduces a `DEFAULT` whose cooked
        // expression is a NULL constant to **no default at all** (18.6 stores no
        // `pg_attrdef` row either), so no column carries a
        // `ColumnDefault::Literal(Value::Null)` — and the store magic moved to `PGMI`,
        // so no legacy store carries one either. `emit_create_table`'s skip stood in
        // for that while such a store could be reopened; it is gone, and this arm is
        // kept only so the match stays total. `DEFAULT NULL` is valid DDL on both
        // engines if some future caller does emit it.
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Int2(n) => n.to_string(),
        Value::Int4(n) => n.to_string(),
        Value::Int8(n) => n.to_string(),
        // A finite float prints as its plain decimal (valid as a bare DEFAULT). A
        // non-finite one (Inf/NaN) has no bare-literal DDL form — fail loud rather
        // than emit invalid SQL. (Unreachable today: the DDL parser only yields
        // finite numeric DEFAULT literals.)
        Value::Float4(n) if n.is_finite() => n.to_string(),
        Value::Float8(n) if n.is_finite() => n.to_string(),
        Value::Float4(_) | Value::Float8(_) => {
            return Err(Error::NotImplemented(
                "cannot render a non-finite float (Inf/NaN) as a portable DEFAULT".into(),
            ));
        }
        // Postgres's own constant typing: a decimal or exponent literal is a
        // `numeric` constant, so `DEFAULT 1.50` keeps its scale (18.6 renders `1.50`
        // in `column_default`, measured) and an out-of-`i64` integer stays exact. The
        // display string is a bare numeric literal, valid DDL on both engines.
        Value::Numeric(n) => n.to_display_string(),
        Value::Text(s) => quote_string_literal(s),
        other => {
            return Err(Error::NotImplemented(format!(
                "cannot render DEFAULT literal {other:?} as portable SQL"
            )));
        }
    })
}

// =============================================================================
// Data
// =============================================================================

/// `COPY <table> (<cols>) FROM stdin;\n<rows>\n\.` — the table's rows in COPY text
/// format (reusing the shared encoder), framed for psql's inline sub-protocol.
fn emit_copy<S: Storage>(schema: &Schema, storage: &S, table: &Table) -> Result<String> {
    let columns: Vec<String> = table.columns.iter().map(|c| quote_ident(&c.name)).collect();
    let params = CopyParams {
        table: &table.name,
        columns: &[], // empty = all columns in catalog order (matches the header)
        format: CopyFormat::Text,
        delimiter: b'\t',
        null_marker: NULL_MARKER,
        header: false,
    };
    let data = execute_copy_to(storage, schema, &params)?;
    // Gated in the server encoding, not merely as Rust UTF-8, which takes a NUL. Emitted
    // raw, a NUL makes a `psql -f` restore refuse on 18.6 and load a different value
    // into pglet (measured). A store can hold one: SQL text handed to an in-process door
    // writes it (`EDGES.md` row 133), and so does a `Value::Text` bound through
    // `Database::execute_bound`. Refusing here keeps loud the export of a row whose COPY
    // text holds one. This gates the COPY payload and not catalog text, where the dump
    // emits a NUL raw (row 133).
    let data = pglet_core::encoding::text_from_utf8(data)?;
    // ...and then the half the payload scan cannot see, over the values themselves — see
    // `refuse_unreadable_values`. Last of the table's gates, so it turns a success into a
    // refusal and re-orders no verdict already reached: a NUL the payload carries raw is
    // answered above it, by `text_from_utf8`. That is the verdict standing above, and the
    // COPY encoder is not a second one — its decline branch cannot be reached for a
    // stored value at all, `value_to_text`'s sole `None` arm being `Value::Null`, which
    // `execute_copy_to` short-circuits ahead of the call.
    //
    // The same walk also asks each stored `date` / `timestamp` whether its text reloads as
    // the value stored — see `temporal_restore_failure` — and answers that second, after
    // the whole table's jsonb verdict, so a jsonb refusal anywhere in the table stands
    // ahead of a temporal one.
    refuse_unreadable_values(storage, table)?;
    Ok(format!(
        "COPY {} ({}) FROM stdin;\n{data}\\.",
        quote_ident(&table.name),
        columns.join(", ")
    ))
}

/// Refuse the export when a stored value in `table` holds a jsonb string the server
/// encoding refuses.
///
/// **The property it serves.** A dump path must not emit a jsonb value 18.6's `jsonb`
/// type refuses, because such a value's `COPY` line fails on the restore *and takes the
/// rest of its table's rows with it* while `psql` still exits `0` — the shape
/// `docs/sequences.md`'s dump invariant asks the export to refuse loudly instead. This
/// gate's width is the encoding scan's (below) and not that whole property: it is what
/// was measured, and a second thing 18.6's `jsonb` refuses would need its own reader.
///
/// **Why the payload scan above does not already answer.** The renderers measured to
/// carry a NUL put the byte itself into the payload — a `text` column's, a `varchar`'s
/// and a `text[]`'s (`EDGES.md` row 133) — so the byte is *in* `data` and
/// [`pglet_core::encoding::text_from_utf8`] finds it. A jsonb string is **escaped** on
/// the way out instead — `escape_json_string` spells a NUL `\u0000` — so the payload
/// holds no NUL to find, and the export walked past it. Five positions were measured: an
/// object key, an object's string value, an array element, a value two containers deep,
/// and a `jsonb[]` element. Each dumped at exit `0`, with each `psql -f` restore into a
/// fresh 18.6 exiting `0`, that table's `COPY` refused
/// `22P05 unsupported Unicode escape sequence` and `count(*)` `0`. Any further renderer
/// that spells a refused byte rather than writing it would need this same treatment;
/// jsonb's is the one measured to.
///
/// **Why it is asked of the value and not of the text.** The rendered text cannot be
/// searched for the escape: a jsonb string whose *content* is the six characters
/// `\u0000` renders as `\\u0000`, which holds `\u0000` as a substring, so a text
/// scan refuses a document 18.6 reads back perfectly. The walk asks each string.
///
/// **Its width is the scan's, not a list of characters.** Each string is handed to
/// [`pglet_core::encoding::utf8_failure_bytes`], the same derivation the payload gate
/// uses, so the reported bytes are `report_invalid_encoding`'s and the gate refuses what
/// the server encoding refuses rather than a character list of its own. Over a Rust
/// `String` that is a NUL and nothing else — a `String` is valid UTF-8 by construction,
/// and this engine's jsonb strings are `String`s.
///
/// **One walk serves two refusals.** The rows are scanned once, and each value is also
/// asked [`temporal_restore_failure`]; the first such failure is held until the scan ends
/// without a jsonb one, so folding the two walks into one scan leaves the order they
/// answer in as it was — every jsonb verdict in the table ahead of any temporal one.
fn refuse_unreadable_values<S: Storage>(storage: &S, table: &Table) -> Result<()> {
    let mut temporal: Option<Error> = None;
    for (_id, row) in storage.scan(&table.name)? {
        for value in &row {
            if let Some(bytes) = value_jsonb_encoding_failure(value) {
                return Err(Error::InvalidEncoding { bytes });
            }
            if temporal.is_none() {
                temporal = temporal_restore_failure(value);
            }
        }
    }
    temporal.map_or(Ok(()), Err)
}

/// The bytes [`Error::InvalidEncoding`] names for the first jsonb string in `value`
/// the server encoding refuses, in rendering order, or `None`.
///
/// A jsonb value is reached through an array element too, `jsonb[]` being a column
/// type here.
fn value_jsonb_encoding_failure(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Jsonb(json) => jsonb_encoding_failure(json),
        Value::Array(arr) => arr.elements.iter().find_map(value_jsonb_encoding_failure),
        _ => None,
    }
}

/// [`value_jsonb_encoding_failure`]'s walk over the jsonb tree: an object's key is
/// asked before its value, as the renderer writes them, and the first failure wins.
fn jsonb_encoding_failure(json: &JsonbValue) -> Option<Vec<u8>> {
    match json {
        JsonbValue::String(s) => pglet_core::encoding::utf8_failure_bytes(s.as_bytes()),
        JsonbValue::Array(items) => items.iter().find_map(jsonb_encoding_failure),
        JsonbValue::Object(pairs) => pairs.iter().find_map(|(key, value)| {
            pglet_core::encoding::utf8_failure_bytes(key.as_bytes())
                .or_else(|| jsonb_encoding_failure(value))
        }),
        JsonbValue::Null | JsonbValue::Bool(_) | JsonbValue::Number(_) => None,
    }
}

/// The refusal the dump raises for the first stored `date` or `timestamp` in `value` — an
/// array's elements included — whose `COPY` text would not reload as the value stored, or
/// `None`. Asked of every stored value by [`refuse_unreadable_values`]'s walk.
///
/// **The property it serves.** The walk reads stored **rows**; a stored expression's text —
/// a `DEFAULT`, a `CHECK`, a partial index's predicate — is not asked (`EDGES.md` row 142).
/// No producer builds an out-of-domain value, and a store written without the domain — one
/// older than the `PGMO` store magic — is refused at open; a value put into storage directly
/// is how the guard is held, and such a value is read back as stored: its `COPY` line renders `0000-12-31`,
/// which 18.6's `date_in` / `timestamp_in` refuse `22008`, taking the table's other rows
/// with it while `psql -f` exits `0` — measured, on a store holding one year-0 `date`,
/// `timestamp` and `timestamptz`. That is the shape `docs/sequences.md`'s dump invariant
/// asks the export to refuse loudly.
///
/// **Three questions, in this order, for a `timestamp`.** The renderer prints an instant's
/// fields at its own offset and appends `+00` for a `timestamptz`, so it is refused when
/// its UTC reading is outside the domain (`22008 timestamp out of range`), when the fields
/// it renders are (the same code), and — inside both — when it is not at UTC, since those
/// fields then reload as a different instant: `2026-06-15 22:00:00-05` renders
/// `2026-06-15 22:00:00+00`, five hours from what is stored. The one route found to such a
/// store is `Database::execute_bound` before that door normalised a bound value.
///
/// An `infinity` or `-infinity` is none of these: its `COPY` text is the word, which both
/// engines read back as the value stored.
fn temporal_restore_failure(value: &Value) -> Option<Error> {
    use pglet_core::types::Infinite;
    match value {
        Value::Date(Infinite::Finite(d)) => pglet_core::types::check_date_domain(*d).err(),
        Value::Timestamp(Infinite::Finite(ts)) => {
            let instant = pglet_core::types::check_timestamp_domain(*ts);
            let rendered = pglet_core::types::check_date_domain(ts.date());
            if instant.is_err() || rendered.is_err() {
                Some(Error::TemporalOutOfRange("timestamp"))
            } else if ts.offset().is_utc() {
                None
            } else {
                Some(Error::Execution(format!(
                    "cannot dump a timestamp stored at UTC offset {}: its text would reload as a different instant",
                    ts.offset()
                )))
            }
        }
        Value::Array(arr) => arr.elements.iter().find_map(temporal_restore_failure),
        _ => None,
    }
}

// =============================================================================
// Constraints + indexes
// =============================================================================

/// How an `Index` round-trips: as a `PRIMARY KEY` / `UNIQUE` constraint, or as a
/// `CREATE [UNIQUE] INDEX`.
enum IndexKind {
    PrimaryKey,
    Unique,
    Plain,
}

/// Classify an index by whether it is the auto-created index behind a `PRIMARY KEY`
/// / `UNIQUE` *constraint* (so it re-emits as `ADD CONSTRAINT`, never as a
/// duplicate `CREATE INDEX`).
///
/// Reads the catalog's constraint marker rather than re-deriving the answer from the
/// index's name. The name convention is still the *mint*-side rule (`<table>_pkey` /
/// `<table>_<columns>_key`, chosen by the executor's namer), but classifying by it here made a
/// deliberately-standalone `CREATE UNIQUE INDEX t_pkey ON t (id)` dump as a `PRIMARY
/// KEY` constraint — a semantic upgrade the restore would then enforce.
const fn classify_index(index: &Index) -> IndexKind {
    match index.constraint_kind() {
        Some(ConstraintKind::PrimaryKey) => IndexKind::PrimaryKey,
        Some(ConstraintKind::Unique) => IndexKind::Unique,
        None => IndexKind::Plain,
    }
}

/// Append a table's `PRIMARY KEY` / `UNIQUE` / `CREATE INDEX` / `CHECK` blocks (in
/// that order, each set name-sorted) to `blocks`.
fn emit_table_constraints(blocks: &mut Vec<String>, schema: &Schema, table: &Table) -> Result<()> {
    let mut indexes: Vec<&Index> = schema
        .indexes
        .values()
        .filter(|ix| ix.table_name == table.name)
        .collect();
    indexes.sort_by(|a, b| a.name.cmp(&b.name));

    // PRIMARY KEY, then UNIQUE constraints, then plain CREATE INDEX — constraint
    // forms first so a later FK can reference the key.
    for ix in indexes
        .iter()
        .filter(|ix| matches!(classify_index(ix), IndexKind::PrimaryKey))
    {
        blocks.push(emit_pk_or_unique(table, ix, ConstraintKind::PrimaryKey)?);
    }
    for ix in indexes
        .iter()
        .filter(|ix| matches!(classify_index(ix), IndexKind::Unique))
    {
        blocks.push(emit_pk_or_unique(table, ix, ConstraintKind::Unique)?);
    }
    for ix in indexes
        .iter()
        .filter(|ix| matches!(classify_index(ix), IndexKind::Plain))
    {
        blocks.push(emit_create_index(table, ix)?);
    }

    // CHECK constraints, each under the name the catalog holds — never a name this
    // emitter invents. A dump is only reloadable if its `ADD CONSTRAINT` names are
    // distinct, and an emitter that counts its own way to a name cannot know what the
    // engine called the constraint: numbering the unnamed checks by ordinal
    // (`<table>_check`, `<table>_check2`, …) collided with a *declared* `<table>_check`
    // on the same table, so `CREATE TABLE dc (a INT, CONSTRAINT dc_check CHECK (a > 0),
    // CHECK (a < 100))` emitted `ADD CONSTRAINT dc_check` twice — refused by stock
    // Postgres **and** by pglet's own restore. Every stored check now carries the name
    // the executor minted, so this reads it.
    for check in &table.checks {
        blocks.push(format!(
            "ALTER TABLE {} ADD CONSTRAINT {} {};",
            quote_ident(&table.name),
            quote_ident(&check.name),
            constraint_def(ConstraintEntryKind::Check(check), RenderMode::Portable)?
        ));
    }
    Ok(())
}

/// `ALTER TABLE <t> ADD CONSTRAINT <name> {PRIMARY KEY|UNIQUE} (<cols>);`
///
/// The definition text is the **one** constraint renderer
/// (`pglet_sql::catalog::constraint_def`), the same string `pg_get_constraintdef`
/// answers — so an export and an introspecting client cannot describe one constraint
/// two ways.
fn emit_pk_or_unique(table: &Table, index: &Index, kind: ConstraintKind) -> Result<String> {
    Ok(format!(
        "ALTER TABLE {} ADD CONSTRAINT {} {};",
        quote_ident(&table.name),
        quote_ident(&index.name),
        constraint_def(
            ConstraintEntryKind::Index(index, kind),
            RenderMode::Portable
        )?
    ))
}

/// `CREATE [UNIQUE] INDEX <name> ON <t> [USING gin] (<keyparts>) [NULLS NOT
/// DISTINCT] [WHERE <pred>];`
fn emit_create_index(table: &Table, index: &Index) -> Result<String> {
    let unique = if index.is_unique() { "UNIQUE " } else { "" };
    let using = if is_gin_index(index, table) {
        " USING gin"
    } else {
        ""
    };
    let key = match &index.key {
        IndexKey::Columns(cols) => cols
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(", "),
        IndexKey::Exprs(exprs) => {
            let parts: Result<Vec<String>> = exprs
                .iter()
                .map(|e| Ok(format!("({})", render_expr(e)?)))
                .collect();
            parts?.join(", ")
        }
    };
    let mut out = format!(
        "CREATE {unique}INDEX {} ON {}{using} ({key})",
        quote_ident(&index.name),
        quote_ident(&table.name),
    );
    if index.nulls_not_distinct {
        out.push_str(" NULLS NOT DISTINCT");
    }
    if let Some(pred) = &index.predicate {
        let _ = write!(out, " WHERE {}", render_expr(pred)?);
    }
    out.push(';');
    Ok(out)
}

/// Whether an index is GIN-backed — the same derivation the executor uses: a
/// non-unique single-bare-column index over a container (array / jsonb) column,
/// the one shape pglet builds only as GIN.
fn is_gin_index(index: &Index, table: &Table) -> bool {
    if index.is_unique() {
        return false;
    }
    index
        .key
        .single_bare_column()
        .and_then(|name| table.columns.iter().find(|c| c.name == name))
        .is_some_and(|col| col.type_oid.is_container())
}

/// `ALTER TABLE <child> ADD CONSTRAINT <name> <def>;` — the FK's definition rendered by
/// the **one** constraint renderer (`pglet_sql::catalog::constraint_def`), byte-identical
/// to what `pg_get_constraintdef` answers for the same constraint.
///
/// That unification moved this emitter's output twice, and both moves were toward 18.6:
/// the referenced relation is now `parent(id)` rather than `parent (id)`, and
/// `ON UPDATE` precedes `ON DELETE` rather than following it. Both spellings reload into
/// stock Postgres, which is exactly why nothing here could see the split — while the fuz
/// spine compares an export's constraint text against an introspecting client's across
/// backends, where a cosmetic split reads as schema drift.
fn emit_foreign_key(table: &Table, fk: &ForeignKey) -> Result<String> {
    Ok(format!(
        "ALTER TABLE {} ADD CONSTRAINT {} {};",
        quote_ident(&table.name),
        quote_ident(&fk.name),
        constraint_def(ConstraintEntryKind::ForeignKey(fk), RenderMode::Portable)?
    ))
}

// =============================================================================
// Deterministic ordering helpers
// =============================================================================

/// The keys of a map, sorted, as borrowed `&str`s — for a deterministic walk over a
/// `HashMap`-keyed catalog section.
fn sorted_keys<'a>(keys: impl Iterator<Item = &'a String>) -> Vec<&'a str> {
    let mut v: Vec<&str> = keys.map(String::as_str).collect();
    v.sort_unstable();
    v
}

/// The schema's tables, sorted by name.
fn sorted_tables(schema: &Schema) -> Vec<&Table> {
    let mut tables: Vec<&Table> = schema.tables.values().collect();
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    // The `Expr` deparser and its per-family name tables live in `pglet_sql::deparse`
    // — one implementation for the dump and `information_schema.columns` alike — but
    // the `parse(render(e)) == e` corpus test stays here, because recording an `Expr`
    // into a catalog needs a `Database`. That cross-crate split is why the three name
    // tables are `pub` rather than `#[cfg(test)]`.
    use pglet_sql::deparse::{
        BUILTIN_FNS, ExtractSpelling, MATH_FUNCS, STRING_FUNCS, VARIADIC_FUNCS, binary_op_sql,
        extract_spelling,
    };
    use pglet_sql::planner::{
        BinaryOp, BuiltinFn, Expr, Literal, MathFunc, StringFunc, Typmod, VariadicFunc,
    };
    use pglet_sql::types::{ArrayValue, PgNumeric};

    /// Build an in-memory CoW-btree `Database` and run each `;`-free statement.
    fn db_with(statements: &[&str]) -> Database {
        let mut db = Database::with_btree();
        for sql in statements {
            db.execute(sql).unwrap();
        }
        db
    }

    #[test]
    fn a_nul_already_in_the_store_refuses_the_dump_rather_than_emitting_it_raw() {
        // This writes the NUL straight into storage, so the test does not depend on which
        // doors write one; SQL text through an in-process door does (`EDGES.md`
        // row 133). Emitted raw, a `psql -f` restore drops the NUL and the rest of its
        // line: 18.6 refuses the file and pglet loads a different value.
        let mut db = db_with(&["CREATE TABLE t (a TEXT)"]);
        db.storage_mut()
            .insert("t", vec![Value::Text("x\0y".into())])
            .unwrap();
        let err = dump_sql(db.schema(), db.storage()).unwrap_err();
        assert_eq!(err.sqlstate(), "22021", "{err}");
        assert_eq!(
            err.to_string(),
            "invalid byte sequence for encoding \"UTF8\": 0x00"
        );

        // The control: a multi-byte character in the same position dumps.
        let mut ok = db_with(&["CREATE TABLE t (a TEXT)"]);
        ok.storage_mut()
            .insert("t", vec![Value::Text("x\u{e9}y".into())])
            .unwrap();
        let dump = dump_sql(ok.schema(), ok.storage()).unwrap();
        assert!(dump.contains("x\u{e9}y\n\\."), "{dump}");
    }

    /// A jsonb string is the position the payload scan above cannot see, because the
    /// renderer escapes the NUL rather than writing it — so the gate asks the value.
    ///
    /// Each position is written straight into storage, so the test does not depend on
    /// which doors reach one; `jsonb_reader_doors.rs` owns the door question. Measured
    /// pre-gate on every position below: the dump exited `0` and each `psql -f` restore
    /// into a fresh 18.6 exited `0` with that table's `COPY` refused
    /// `22P05 unsupported Unicode escape sequence` and `count(*)` `0` — the clean
    /// neighbour row in the same table lost with it.
    #[test]
    fn a_nul_inside_a_stored_jsonb_string_refuses_the_dump_the_payload_scan_walks_past() {
        let nul = JsonbValue::String("x\0y".into());
        let positions: Vec<(&str, JsonbValue)> = vec![
            ("a scalar string", nul.clone()),
            (
                "an object value",
                JsonbValue::Object(vec![("k".into(), nul.clone())]),
            ),
            (
                "an object key",
                JsonbValue::Object(vec![(
                    "x\0y".into(),
                    JsonbValue::Number(PgNumeric::from_i128(1)),
                )]),
            ),
            ("an array element", JsonbValue::Array(vec![nul.clone()])),
            (
                "a nested object",
                JsonbValue::Object(vec![(
                    "outer".into(),
                    JsonbValue::Array(vec![JsonbValue::Object(vec![(
                        "inner".into(),
                        nul.clone(),
                    )])]),
                )]),
            ),
        ];
        for (what, json) in positions {
            let mut db = db_with(&["CREATE TABLE j (id INT, v JSONB)"]);
            // A clean row beside it: the restore loses this one too, which is why the
            // export refusing is better than the dump succeeding.
            db.storage_mut()
                .insert("j", vec![Value::Int4(9), Value::Jsonb(JsonbValue::Null)])
                .unwrap();
            db.storage_mut()
                .insert("j", vec![Value::Int4(1), Value::Jsonb(json)])
                .unwrap();
            let err = dump_sql(db.schema(), db.storage()).unwrap_err();
            assert_eq!(err.sqlstate(), "22021", "{what}: {err}");
            assert_eq!(
                err.to_string(),
                "invalid byte sequence for encoding \"UTF8\": 0x00",
                "{what}"
            );
        }

        // A `jsonb[]` element reaches the walk through the array arm.
        let mut arr = db_with(&["CREATE TABLE ja (v JSONB[])"]);
        arr.storage_mut()
            .insert(
                "ja",
                vec![Value::Array(ArrayValue::new(
                    TypeOid::Jsonb,
                    vec![Value::Jsonb(JsonbValue::Array(vec![nul]))],
                ))],
            )
            .unwrap();
        let err = dump_sql(arr.schema(), arr.storage()).unwrap_err();
        assert_eq!(err.sqlstate(), "22021", "a jsonb[] element: {err}");
    }

    /// The other polarity: what the gate must **not** refuse.
    ///
    /// The `\u0000` spelling is the trap — a jsonb string whose *content* is those six
    /// characters renders as `\\u0000`, which holds the escape as a substring, so a
    /// gate that searched the rendered text would refuse a document both engines read
    /// back. Measured on 18.6: this dump restores with every row at exit `0`, and
    /// `'{"k": "a\\u0000b"}'::jsonb::text` there is `{"k": "a\\u0000b"}`, pglet's
    /// value byte for byte.
    #[test]
    fn the_jsonb_gate_refuses_no_document_both_engines_read_back() {
        // Built from the byte rather than written as an escape, so nothing between the
        // source and the store can decode it.
        let bs = char::from(92u8);
        let literal_escape = format!("a{bs}u0000b");
        let mut db = db_with(&["CREATE TABLE j (id INT, v JSONB)"]);
        let rows = vec![
            JsonbValue::Object(vec![(
                "k".into(),
                JsonbValue::String(literal_escape.clone()),
            )]),
            JsonbValue::Object(vec![(literal_escape, JsonbValue::Bool(true))]),
            JsonbValue::String("a\u{e9}b \u{1f600}".into()),
            JsonbValue::Array(vec![
                JsonbValue::Null,
                JsonbValue::String("\u{7f}\t\n".into()),
                JsonbValue::Number(PgNumeric::from_i128(12)),
            ]),
        ];
        for (i, json) in rows.into_iter().enumerate() {
            let id = i32::try_from(i).unwrap();
            db.storage_mut()
                .insert("j", vec![Value::Int4(id), Value::Jsonb(json)])
                .unwrap();
        }
        let dump = dump_sql(db.schema(), db.storage()).expect("the gate refuses none of these");
        // The six-character content survives into the COPY text, doubled once by
        // jsonb's own escape and once by COPY's.
        assert!(dump.contains(&format!("{bs}{bs}{bs}{bs}u0000b")), "{dump}");
    }

    #[test]
    fn tiny_table_exact_shape() {
        // A single PK + NOT NULL table with two rows: pins the emitter's exact
        // output so an emitter regression fails here without needing Postgres.
        let db = db_with(&[
            "CREATE TABLE t (id INT PRIMARY KEY, name TEXT NOT NULL)",
            "INSERT INTO t VALUES (1, 'a'), (2, 'b')",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        let expected = "\
CREATE TABLE t (
    id integer NOT NULL,
    name text NOT NULL
);

COPY t (id, name) FROM stdin;
1\ta
2\tb
\\.

ALTER TABLE t ADD CONSTRAINT t_pkey PRIMARY KEY (id);
";
        assert_eq!(dump, expected);
    }

    #[test]
    fn a_standalone_index_named_like_a_constraint_stays_an_index() {
        // The classification reads the catalog's constraint marker, not the index's
        // name. A deliberately standalone `CREATE UNIQUE INDEX t_pkey` is an index —
        // dumping it as `ADD CONSTRAINT … PRIMARY KEY` would silently upgrade it to a
        // constraint (NOT NULL columns, an undroppable index) on restore.
        let db = db_with(&[
            "CREATE TABLE t (id INT, k INT)",
            "CREATE UNIQUE INDEX t_pkey ON t (id)",
            "CREATE UNIQUE INDEX t_k_key ON t (k)",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(
            dump.contains("CREATE UNIQUE INDEX t_pkey ON t (id);"),
            "{dump}"
        );
        assert!(
            dump.contains("CREATE UNIQUE INDEX t_k_key ON t (k);"),
            "{dump}"
        );
        assert!(!dump.contains("ADD CONSTRAINT"), "{dump}");
    }

    #[test]
    fn a_declared_constraint_name_round_trips_through_the_dump() {
        // The constraint's name is its index's name, so the dump emits what the user
        // declared. Emitting the `<table>_pkey` convention instead would rename every
        // key on restore — silently, and in a way an introspecting client cannot tell
        // from the original.
        let db = db_with(&[
            "CREATE TABLE t (a INT NOT NULL, b INT, CONSTRAINT my_pk PRIMARY KEY (a), \
             CONSTRAINT my_uq UNIQUE (b))",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(
            dump.contains("ALTER TABLE t ADD CONSTRAINT my_pk PRIMARY KEY (a);"),
            "{dump}"
        );
        assert!(
            dump.contains("ALTER TABLE t ADD CONSTRAINT my_uq UNIQUE (b);"),
            "{dump}"
        );
        assert!(!dump.contains("t_pkey"), "{dump}");
    }

    #[test]
    fn escaping_default_and_array_shape() {
        // A reserved-word column name (`order`) is quoted; a tab in a text value is
        // COPY-escaped; a text[] renders `{…}`; a DEFAULT and a NULL round-trip.
        let db = db_with(&[
            "CREATE TABLE g (id INT, \"order\" INT DEFAULT 0, tags TEXT[], note TEXT)",
            "INSERT INTO g VALUES (1, 5, '{x,y}', 'a\tb'), (2, DEFAULT, NULL, NULL)",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        let expected = "\
CREATE TABLE g (
    id integer,
    \"order\" integer DEFAULT 0,
    tags text[],
    note text
);

COPY g (id, \"order\", tags, note) FROM stdin;
1\t5\t{x,y}\ta\\tb
2\t0\t\\N\t\\N
\\.
";
        assert_eq!(dump, expected);
    }

    #[test]
    fn enum_and_sequence_and_default_fns() {
        let db = db_with(&[
            "CREATE TYPE mood AS ENUM ('happy', 'sad')",
            "CREATE TABLE e (id SERIAL PRIMARY KEY, m mood NOT NULL DEFAULT 'happy')",
            "INSERT INTO e (m) VALUES ('happy'), ('sad')",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // The sequence precedes the ENUM precedes the table (a sequence and an enum may
        // share a name, and only that order reloads it); SERIAL emits a sequence +
        // nextval default.
        assert!(
            dump.starts_with(
                "CREATE SEQUENCE e_id_seq;\n\nCREATE TYPE mood AS ENUM ('happy', 'sad');\n\n"
            ),
            "{dump}"
        );
        // The sequence's *declared start* (1, so no clause) — not its counter, which
        // the folded `START WITH <counter + 1>` used to smuggle through here.
        assert!(dump.contains("CREATE SEQUENCE e_id_seq;"));
        // A SERIAL's default is hoisted out of the `CREATE TABLE` like every other
        // `nextval` one, so no table's creation depends on a sequence existing yet.
        assert!(
            !dump.contains("id integer NOT NULL DEFAULT nextval"),
            "{dump}"
        );
        assert!(
            dump.contains(
                "ALTER TABLE e ALTER COLUMN id SET DEFAULT nextval('e_id_seq'::regclass);"
            )
        );
        // A non-sequence default stays inline.
        assert!(dump.contains("m mood NOT NULL DEFAULT 'happy'"));
        assert!(dump.contains("ALTER TABLE e ADD CONSTRAINT e_pkey PRIMARY KEY (id);"));
        // A SERIAL column's sequence is owned by it, emitted after the table.
        assert!(dump.contains("ALTER SEQUENCE e_id_seq OWNED BY e.id;"));
        // ...and the counter travels separately, at the very end: two ids were issued,
        // so the next `nextval` must be 3.
        assert!(
            dump.trim_end()
                .ends_with("SELECT setval('e_id_seq', 3, false);")
        );
        // Ordering: the CREATE precedes the OWNED BY precedes the setval.
        let create = dump.find("CREATE SEQUENCE e_id_seq;").unwrap();
        let owned = dump.find("ALTER SEQUENCE e_id_seq OWNED BY").unwrap();
        let setval = dump.find("SELECT setval('e_id_seq'").unwrap();
        let create_table = dump.find("CREATE TABLE e (").unwrap();
        assert!(create < create_table, "the sequence precedes its table");
        assert!(
            create_table < owned,
            "OWNED BY names the table, so it follows it"
        );
        assert!(owned < setval, "the counter is restored after the data");
    }

    #[test]
    fn a_name_needing_quotes_carries_its_quoting_inside_both_literals() {
        // The emitter half of the two-grammar rule, asserted **hermetically**. A
        // sequence name reaches the script in an identifier position and in a string
        // literal inside a `regclass` cast, and a literal is read back by
        // `SplitIdentifierString`, which downcases an unquoted part — so a name that
        // is not a safe bare identifier must carry its *identifier* quoting inside the
        // string or the two positions name different relations.
        //
        // Its live guards need a real Postgres and a corpus cell that happens to hold
        // a quoted name, which put the whole emitter fix outside `cargo test
        // --workspace` — a revert of it with the corpus cells removed was caught by
        // nothing. This is the cheapest gate that sees it, and it is corpus-independent.
        let db = db_with(&[
            "CREATE SEQUENCE \"Odd_seq\" START WITH 40",
            "CREATE TABLE q (id INT DEFAULT nextval('\"Odd_seq\"'), v TEXT)",
            "INSERT INTO q (v) VALUES ('a')",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // Both literal positions, and they fail independently: an identity column emits
        // no `nextval` at all, so its counter travels through the `setval` alone.
        assert!(
            dump.contains(
                "ALTER TABLE q ALTER COLUMN id SET DEFAULT nextval('\"Odd_seq\"'::regclass);"
            ),
            "{dump}"
        );
        assert!(
            dump.contains("SELECT setval('\"Odd_seq\"', 41, false);"),
            "{dump}"
        );
        // The identifier positions spell the same relation — the whole point of routing
        // the literal through `quote_ident` rather than a second predicate.
        assert!(
            dump.contains("CREATE SEQUENCE \"Odd_seq\" START WITH 40;"),
            "{dump}"
        );
        // ...and the bare spelling, which names `odd_seq` on Postgres, appears nowhere.
        assert!(!dump.contains("nextval('Odd_seq'"), "{dump}");
        assert!(!dump.contains("setval('Odd_seq'"), "{dump}");
    }

    #[test]
    fn a_backslash_in_a_name_forces_the_escape_string_form_in_both_literals() {
        // The second SQL-string grammar the emitter can reach, and the one a scan for
        // `nextval('` cannot see. A backslash is the one byte whose meaning depends on
        // `standard_conforming_strings`, so `quote_string_literal` emits `E'…'` for it
        // — which means a name reaches the script in *three* grammars, not two, and any
        // reader of the emitted script must know all three.
        let db = db_with(&[
            "CREATE SEQUENCE \"a\\b\" START WITH 6",
            "CREATE TABLE bs (id INT DEFAULT nextval('\"a\\b\"'), v TEXT)",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(
            dump.contains(
                "ALTER TABLE bs ALTER COLUMN id SET DEFAULT nextval(E'\"a\\\\b\"'::regclass);"
            ),
            "{dump}"
        );
        assert!(
            dump.contains("SELECT setval(E'\"a\\\\b\"', 6, false);"),
            "{dump}"
        );
        // The identifier position takes no backslash escaping — `\` is an ordinary
        // character inside a double-quoted identifier on both engines — so the two
        // grammars spell the same name differently *and correctly*.
        assert!(
            dump.contains("CREATE SEQUENCE \"a\\b\" START WITH 6;"),
            "{dump}"
        );
    }

    #[test]
    fn owned_by_none_stops_the_dump_emitting_an_owner() {
        // What this pins is the *rendering* of a detached sequence, not the detach —
        // `TRUNCATE t RESTART IDENTITY` restarts only **owned** sequences, so the
        // detach has a plain-SQL observable of its own
        // (`sequences/truncate_restart_identity_after_owned_by_none.sql`).
        let db = db_with(&[
            "CREATE TABLE dn (a SERIAL, b INT)",
            "ALTER SEQUENCE dn_a_seq OWNED BY NONE",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(
            dump.contains("CREATE SEQUENCE dn_a_seq;"),
            "the sequence itself survives the detach"
        );
        assert!(
            !dump.contains("OWNED BY"),
            "the detached sequence has no owner to emit:\n{dump}"
        );
    }

    #[test]
    fn a_dangling_sequence_owner_is_not_emitted() {
        // A sequence outliving its owner renders no `ALTER SEQUENCE … OWNED BY` — a
        // statement naming a relation the script never creates, which stock Postgres
        // rejects and pglet's own restore rejects too.
        //
        // **No route reaches the state any more**, and the shape below is what that
        // costs a guard. `DROP TABLE` and `ALTER TABLE … DROP COLUMN` take an owned
        // sequence with them, so the two `DROP` routes this test used to seed with are
        // asserted from the other side — the sequence is *gone*, not orphaned — and the
        // third seed, an `ADD COLUMN … SERIAL` failing *after* registering its
        // sequence, is closed too: the registration is staged behind the backfill
        // validation now (`executor::ddl`'s `staged_sequences`), so the failed statement
        // leaves no sequence at all. Its assertions are inverted the same way. A drop or
        // an `ADD COLUMN` that regressed to leaking fails on those rather than silently
        // becoming this test's only live seed.
        //
        // So the emitter branch is reached from a **hand-built catalog** — the same
        // argument `FkRef`'s guard makes: the property is that an unresolvable owner
        // renders no `OWNED BY`, and a guard that can only produce the state through a
        // defect stops testing the emitter the moment the defect is fixed.
        //
        // The enumeration behind "no route" is `owned_sequences`: a `DROP` removes every
        // sequence whose `owner` names the dropped table or column, so no `DROP` can
        // orphan one; the remaining registration sites (`CREATE TABLE`, `ADD COLUMN`,
        // `ALTER COLUMN … ADD GENERATED`) all land their sequence with the owner already
        // in the catalog, or not at all. The enumeration is over writers of the **owner
        // edge**, not creators of a sequence, so it takes two more members: `CREATE
        // SEQUENCE … OWNED BY` and `ALTER SEQUENCE … OWNED BY`, both of which build the
        // record through `executor::ddl`'s `resolve_sequence_owner` — a wrong-kind owner
        // (`42809`), a missing table (`42P01`) and a missing column (`42703`) are all
        // raised before a `SequenceOwner` exists, so neither can record one that does not
        // resolve. The `owned_sequences` half survived D7b's identity-detach bug intact —
        // its routes auto-dropped the sequence rather than orphaning it. What that bug
        // reached instead was this state's mirror image, a **column default** naming a
        // gone sequence, which no test here seeds and no assertion here looks for; the
        // dump-level property is stated in `pglet_tests`'
        // `check_every_nextval_has_a_sequence`.
        let mut db = db_with(&[
            "CREATE TABLE dh (a SERIAL, b INT)",
            "DROP TABLE dh",
            "CREATE TABLE dc (a SERIAL, b INT)",
            "ALTER TABLE dc DROP COLUMN a",
            // A live owner, to prove the skip is discriminating rather than blanket.
            "CREATE TABLE live (a SERIAL, b INT)",
            "CREATE TABLE fa (x INT)",
            "INSERT INTO fa VALUES (1)",
        ]);
        // The closed route: a failed `ADD COLUMN … SERIAL` leaves neither the column nor
        // the sequence, so it orphans nothing.
        assert!(
            db.execute("ALTER TABLE fa ADD COLUMN c SERIAL CHECK (c > 100)")
                .is_err()
        );
        assert!(
            !db.schema().sequences.contains_key("fa_c_seq"),
            "the failed ADD COLUMN left a phantom sequence — it used to be this test's \
             only live seed for a dangling owner"
        );
        // The hand-built seeds for the emitter branch, and there are **two** because the
        // owner is a `(table, column)` pair and the emitter's test reads both halves:
        // `get_table(owner.table).is_some_and(|t| t.columns.iter().any(…))`. A gone
        // *table* alone leaves the column half untested — narrowing the check to a bare
        // `.is_some()` still skips it, so that mutation passes. The second seed names a
        // table the catalog **does** hold (`fa`, whose `ADD COLUMN c` was refused above)
        // and a column it does not, which is the only shape the two spellings answer
        // differently. Reachable only here, which is the point.
        db.schema_mut().sequences.insert(
            "orphan_seq".to_string(),
            Sequence::owned_by(1, "gone_table", "gone_column"),
        );
        db.schema_mut().sequences.insert(
            "orphan_col_seq".to_string(),
            Sequence::owned_by(1, "fa", "c"),
        );
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        for orphan in ["orphan_seq", "orphan_col_seq"] {
            assert!(
                dump.contains(&format!("CREATE SEQUENCE {orphan};")),
                "{orphan} is still a sequence:\n{dump}"
            );
            assert!(
                !dump.contains(&format!("ALTER SEQUENCE {orphan} OWNED BY")),
                "{orphan}'s owner does not resolve, so no OWNED BY:\n{dump}"
            );
        }
        // The three closed routes, asserted from the other side: two went with their
        // owner and one was never registered, so none reaches the dump at all.
        for absent in ["dh_a_seq", "dc_a_seq", "fa_c_seq"] {
            assert!(
                !dump.contains(absent),
                "{absent} must not be in the dump at all:\n{dump}"
            );
        }
        assert!(
            dump.contains("ALTER SEQUENCE live_a_seq OWNED BY live.a;"),
            "a resolvable owner is still emitted:\n{dump}"
        );
    }

    #[test]
    fn a_below_one_sequence_declares_the_minvalue_it_needs() {
        // pglet accepts a start / counter below 1 (it models no MINVALUE); stock
        // Postgres restores one only under an explicit floor.
        let db = db_with(&[
            "CREATE SEQUENCE zs START WITH 0",
            "CREATE SEQUENCE ns",
            "SELECT setval('ns', -5)",
            "CREATE SEQUENCE ok",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // The declared start is the floor here, and the explicit `START WITH` rides
        // along even though it is not `1`'s default-omission case.
        assert!(
            dump.contains("CREATE SEQUENCE zs MINVALUE 0 START WITH 0;"),
            "a zero start declares its floor:\n{dump}"
        );
        assert!(dump.contains("SELECT setval('zs', 0, false);"));
        // Here the *counter* is the floor and the start is the default `1` — which
        // must still be spelled out, or `init_params` seeds `seqstart` from the
        // MINVALUE and the restored sequence answers a bare RESTART with `-4`.
        assert!(
            dump.contains("CREATE SEQUENCE ns MINVALUE -4 START WITH 1;"),
            "a negative counter declares its floor and pins the start:\n{dump}"
        );
        assert!(dump.contains("SELECT setval('ns', -4, false);"));
        // An ordinary sequence is untouched — no MINVALUE noise.
        assert!(
            dump.contains("CREATE SEQUENCE ok;"),
            "an in-range sequence keeps its bare form:\n{dump}"
        );
    }

    #[test]
    fn foreign_key_and_unique_and_check() {
        let db = db_with(&[
            "CREATE TABLE parent (id INT PRIMARY KEY, code TEXT UNIQUE)",
            "CREATE TABLE child (id INT PRIMARY KEY, pid INT REFERENCES parent(id) ON DELETE CASCADE, qty INT CHECK (qty > 0))",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // UNIQUE constraint re-emits as ADD CONSTRAINT (not a duplicate CREATE INDEX).
        assert!(dump.contains("ALTER TABLE parent ADD CONSTRAINT parent_code_key UNIQUE (code);"));
        assert!(!dump.contains("CREATE INDEX"));
        assert!(!dump.contains("CREATE UNIQUE INDEX"));
        // The CHECK renders its predicate.
        assert!(dump.contains("ADD CONSTRAINT child_qty_check CHECK ((qty > 0));"));
        // The FK is emitted last, after all PK/UNIQUE, with its ON DELETE action.
        assert!(dump.contains(
            "ALTER TABLE child ADD CONSTRAINT child_pid_fkey FOREIGN KEY (pid) REFERENCES parent(id) ON DELETE CASCADE;"
        ));
        let fk_pos = dump.find("FOREIGN KEY").unwrap();
        let pk_pos = dump.find("PRIMARY KEY").unwrap();
        assert!(fk_pos > pk_pos, "FK must come after PK/UNIQUE");
    }

    #[test]
    fn foreign_key_emits_both_on_delete_and_on_update_actions() {
        let db = db_with(&[
            "CREATE TABLE parent (id INT PRIMARY KEY)",
            "CREATE TABLE child (id INT PRIMARY KEY, \
             pid INT REFERENCES parent(id) ON DELETE SET NULL ON UPDATE CASCADE)",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // Both non-default actions render, ON DELETE before ON UPDATE.
        assert!(dump.contains(
            "ALTER TABLE child ADD CONSTRAINT child_pid_fkey FOREIGN KEY (pid) REFERENCES \
             parent(id) ON UPDATE CASCADE ON DELETE SET NULL;"
        ));
    }

    #[test]
    fn foreign_key_omits_default_no_action_on_update() {
        // ON UPDATE NO ACTION is the default on both engines and must not be emitted.
        let db = db_with(&[
            "CREATE TABLE parent (id INT PRIMARY KEY)",
            "CREATE TABLE child (id INT PRIMARY KEY, \
             pid INT REFERENCES parent(id) ON UPDATE RESTRICT)",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(dump.contains(
            "ALTER TABLE child ADD CONSTRAINT child_pid_fkey FOREIGN KEY (pid) REFERENCES \
             parent(id) ON UPDATE RESTRICT;"
        ));
        assert!(
            !dump.contains("ON DELETE"),
            "default ON DELETE NO ACTION is omitted"
        );
    }

    #[test]
    fn partial_index_round_trips_as_create_index() {
        let db = db_with(&[
            "CREATE TABLE u (id INT PRIMARY KEY, email TEXT, deleted_at TIMESTAMPTZ)",
            "CREATE UNIQUE INDEX u_email_live ON u (lower(email)) WHERE deleted_at IS NULL",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(dump.contains(
            "CREATE UNIQUE INDEX u_email_live ON u ((lower(email))) WHERE (deleted_at IS NULL);"
        ));
    }

    #[test]
    fn persisted_unary_plus_renders_at_every_stored_expression_site() {
        // `Expr::UnaryPlus` is persisted-reachable through all three stored-expression
        // sites, and `render_expr`'s former catch-all declined it — bricking
        // `dump_sql` for the **whole database**, not just the offending table. All
        // three must render.
        //
        // The spelling is Postgres's own: `information_schema.columns.column_default`
        // and `pg_get_constraintdef` both render unary `+` as `(+ x)`, and a real
        // Postgres accepts this dump and echoes back `CHECK ((a = (+ 1)))` verbatim.
        //
        // Rendering faithfully rather than folding to the bare operand is deliberate.
        // The wrapper is semantically transparent, so `1` would evaluate the same —
        // but the persisted *shape* is what the planner's partial-index implication
        // check compares structurally, so dropping the `+` on restore would silently
        // change which queries a `WHERE +a > 0` index serves.
        let db = db_with(&[
            "CREATE TABLE t (a INT CHECK (a = +1))",
            "CREATE TABLE u (a INT)",
            "CREATE INDEX ixk ON u ((+a))",
            "CREATE TABLE w (a INT)",
            "CREATE INDEX ixp ON w (a) WHERE +a > 0",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        assert!(
            dump.contains("ADD CONSTRAINT t_a_check CHECK ((a = (+ 1)));"),
            "CHECK predicate: {dump}"
        );
        assert!(
            dump.contains("CREATE INDEX ixk ON u (((+ a)));"),
            "index key expression: {dump}"
        );
        assert!(
            dump.contains("CREATE INDEX ixp ON w (a) WHERE ((+ a) > 0);"),
            "index partial predicate: {dump}"
        );
    }

    #[test]
    fn persisted_unary_plus_dump_round_trips_through_pglet() {
        // Reloading the dump must rebuild the same catalog — so a second dump is
        // byte-identical, and the restored CHECK is live rather than decoration.
        let db = db_with(&[
            "CREATE TABLE t (a INT CHECK (a = +1))",
            "CREATE TABLE w (a INT)",
            "CREATE INDEX ixp ON w (a) WHERE +a > 0",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();

        // Replay the DDL. The `COPY … FROM stdin` blocks (emitted per table even with
        // no rows) go through the wire `copy_in` path, not `execute`, so drop each
        // one from its `COPY` line through its `\.` terminator; the schema is what
        // this test is about.
        let mut ddl = String::new();
        let mut in_copy = false;
        for line in dump.lines() {
            if in_copy {
                in_copy = line != "\\.";
            } else if line.starts_with("COPY ") {
                in_copy = true;
            } else {
                ddl.push_str(line);
                ddl.push('\n');
            }
        }

        let mut restored = Database::with_btree();
        for statement in ddl.split(";\n").map(str::trim).filter(|s| !s.is_empty()) {
            restored.execute(statement).unwrap();
        }
        assert_eq!(
            dump_sql(restored.schema(), restored.storage()).unwrap(),
            dump,
            "re-dumping a restored database must be byte-identical"
        );
        // The restored CHECK is live, so the predicate survived as a predicate.
        restored.execute("INSERT INTO t VALUES (1)").unwrap();
        assert!(restored.execute("INSERT INTO t VALUES (2)").is_err());
    }

    #[test]
    fn signed_numeric_column_defaults_round_trip() {
        // `DEFAULT -7` / `DEFAULT +7` were rejected outright by the DDL parser (a
        // token-level classifier that never accepted a sign), so this is the first
        // coverage that they parse, store, and dump.
        let db = db_with(&["CREATE TABLE s (a INT DEFAULT -7, b INT DEFAULT +7)"]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // The sign is folded into the stored `Value`, so both render as constants.
        assert!(dump.contains("a integer DEFAULT -7"), "{dump}");
        assert!(dump.contains("b integer DEFAULT 7"), "{dump}");
    }

    #[test]
    fn unsupported_default_fails_loud() {
        // A literal-only `render_default_literal` rejects a value shape it can't
        // reproduce — guarding the correct-or-loud contract directly.
        let err = render_default_literal(&Value::Uuid([0; 16]));
        assert!(err.is_err());
    }

    // =========================================================================
    // `parse(render(e)) == e` — the round-trip property
    // =========================================================================

    /// The column set every round-trip fixture's expression is written against.
    const ROUND_TRIP_COLUMNS: &str = "a INT, n NUMERIC, s TEXT, ts TIMESTAMP, u UUID, m rt_mood";

    /// The enum every round-trip fixture's database holds, for the corpus's cast to one.
    const ROUND_TRIP_ENUM: &str = "CREATE TYPE rt_mood AS ENUM ('x', 'y')";

    /// One of the four places a DDL statement persists an `Expr` into the catalog —
    /// the only four [`render_expr`] is reachable from.
    #[derive(Clone, Copy, Debug)]
    enum StoredExprSite {
        Check,
        IndexKey,
        IndexPredicate,
        /// A column `DEFAULT` — the **fourth** persistence site, and the one whose
        /// corpus entry has to be spelled differently: a default is *closed* (a
        /// column reference in one is `0A000` on both engines), so its column is
        /// dropped from the statement and the corpus's `a` / `s` / `ts` leaves are
        /// substituted with constants of the same type. What that buys is real
        /// coverage rather than a fourth name for the same walk — a default is the
        /// one persisted expression the parser reaches through *two* routes (the
        /// column-constraint `b_expr` and the `SET DEFAULT` `a_expr`) and normalizes
        /// against the ten closed arms, so a spelling that round-trips at the other
        /// three sites can still land on the wrong arm here.
        ColumnDefault,
    }

    impl StoredExprSite {
        const ALL: [Self; 4] = [
            Self::Check,
            Self::IndexKey,
            Self::IndexPredicate,
            Self::ColumnDefault,
        ];

        /// Record `expr_sql` at this site in a fresh database and return the `Expr`
        /// the catalog stored — the exact value `dump_sql` would hand `render_expr`.
        fn store(self, expr_sql: &str) -> Expr {
            let mut db = Database::with_btree();
            // The enum a corpus entry casts to (`StringFunc::EnumIn`): a cast's target
            // resolves against the catalog, so it has to exist before the DDL does.
            db.execute(ROUND_TRIP_ENUM).unwrap();
            match self {
                Self::Check => {
                    db.execute(&format!(
                        "CREATE TABLE t ({ROUND_TRIP_COLUMNS}, CHECK ({expr_sql}))"
                    ))
                    .unwrap_or_else(|e| panic!("CHECK ({expr_sql}): {e}"));
                    db.schema().tables["t"].checks[0].expr.clone()
                }
                Self::IndexKey => {
                    db.execute(&format!("CREATE TABLE t ({ROUND_TRIP_COLUMNS})"))
                        .unwrap();
                    db.execute(&format!("CREATE INDEX ix ON t (({expr_sql}))"))
                        .unwrap_or_else(|e| panic!("INDEX (({expr_sql})): {e}"));
                    match &db.schema().indexes["ix"].key {
                        IndexKey::Exprs(parts) => parts[0].clone(),
                        IndexKey::Columns(cols) => {
                            panic!("({expr_sql}) stored as a bare-column key {cols:?}")
                        }
                    }
                }
                Self::IndexPredicate => {
                    db.execute(&format!("CREATE TABLE t ({ROUND_TRIP_COLUMNS})"))
                        .unwrap();
                    db.execute(&format!("CREATE INDEX ix ON t (a) WHERE {expr_sql}"))
                        .unwrap_or_else(|e| panic!("INDEX … WHERE {expr_sql}: {e}"));
                    db.schema().indexes["ix"]
                        .predicate
                        .clone()
                        .unwrap_or_else(|| panic!("WHERE {expr_sql} stored no predicate"))
                }
                Self::ColumnDefault => {
                    let closed = close_over_columns(expr_sql);
                    db.execute(&format!("CREATE TABLE t (d BOOL DEFAULT ({closed}))"))
                        .unwrap_or_else(|e| panic!("DEFAULT ({closed}): {e}"));
                    match db.schema().tables["t"].columns[0].default.clone() {
                        Some(ColumnDefault::Expr(expr)) => expr,
                        other => panic!("DEFAULT ({closed}) stored {other:?}, not an Expr"),
                    }
                }
            }
        }
    }

    /// `CREATE INDEX` over `expr_sql` at an index `site`, as [`StoredExprSite::store`]
    /// issues it, answering the statement's result rather than panicking on a refusal.
    fn try_store_index(site: StoredExprSite, expr_sql: &str) -> Result<()> {
        let mut db = Database::with_btree();
        db.execute(ROUND_TRIP_ENUM).unwrap();
        db.execute(&format!("CREATE TABLE t ({ROUND_TRIP_COLUMNS})"))
            .unwrap();
        let sql = match site {
            StoredExprSite::IndexPredicate => {
                format!("CREATE INDEX ix ON t (a) WHERE {expr_sql}")
            }
            _ => format!("CREATE INDEX ix ON t (({expr_sql}))"),
        };
        db.execute(&sql).map(|_| ())
    }

    /// Substitute a constant of the same type for each `ROUND_TRIP_COLUMNS` leaf, so
    /// a corpus entry written against those columns can be a **column `DEFAULT`** —
    /// which admits no column reference at all (`0A000` on both engines).
    ///
    /// Textual, and deliberately so: the point is to reach `render_expr` with the same
    /// *node shapes* the other three sites reach it with, and a constant of the right
    /// type produces exactly those. Word boundaries keep `a` out of `abs` and `s` out
    /// of `split_part`.
    fn close_over_columns(expr_sql: &str) -> String {
        // A qualified reference is a column reference too, so drop the qualifier
        // before the word walk rather than leaving `t.1` behind.
        let expr_sql = &expr_sql.replace("t.", "");
        let mut out = String::new();
        let bytes = expr_sql.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i == start {
                out.push(bytes[i] as char);
                i += 1;
                continue;
            }
            out.push_str(match &expr_sql[start..i] {
                "a" => "1",
                "n" => "1.5",
                "s" => "'x'",
                "ts" => "'2024-01-15 12:30:45'::timestamp",
                "u" => "'00000000-0000-4000-8000-000000000001'::uuid",
                "m" => "'x'::rt_mood",
                word => word,
            });
        }
        out
    }

    /// Every renderable `Expr` variant, as the SQL that persists it. Boolean-valued
    /// so one corpus drives all four sites (a `CHECK`, a partial predicate and a
    /// `BOOL` column's `DEFAULT` must each be a predicate; an index key may be
    /// anything, and a boolean is indexable).
    /// Each function family therefore rides as a sub-expression, which is what the
    /// round-trip has to preserve anyway.
    ///
    /// **This list is enforced, not trusted.** Nothing about a `match` arm or a name
    /// table forces a line here, so
    /// [`round_trip_corpus_covers_every_renderable_form`] reads back what these
    /// entries actually store and fails when a renderable variant, `Literal`, or
    /// function-family member is not among them.
    /// The [`StringFunc`] members no corpus entry can reach, and why.
    ///
    /// `pg_advisory_xact_lock` answers only as a top-level `SELECT` item, where the host
    /// has taken the writer for it; a stored expression is evaluated by the ordinary
    /// evaluator, which refuses a non-NULL call (`0A000`), so the DDL-time probe refuses
    /// the expression.
    ///
    /// The other three read the **catalog**, which a stored expression's DDL-time probe has
    /// none of to lend (`pg_get_serial_sequence` looks its table up), so `CHECK (pg_get_constraintdef(a) IS NOT NULL)` and
    /// `CHECK ((a)::regclass IS NOT NULL)` are refused here (`0A000`) where 18.6 accepts
    /// both — a recorded over-refusal, and a loud one. Refusing at DDL is the deliberate
    /// half: accepting would persist an expression that then fails `0A000` on **every
    /// insert**, since the write path's evaluation environment has no catalog either.
    ///
    /// Skipping them keeps this guard's shape intact rather than weakening it — a *new*
    /// `StringFunc` still fails the check until someone classifies it — and their two
    /// render arms are proven directly by `deparse`'s
    /// `catalog_reading_string_fns_render_to_their_written_form`.
    const NOT_STORABLE_STRING_FUNCS: &[StringFunc] = &[
        StringFunc::PgGetConstraintdef,
        StringFunc::Regclass,
        StringFunc::AdvisoryXactLock,
        StringFunc::PgGetSerialSequence,
    ];

    /// The [`ROUND_TRIP_CORPUS`] entries no index can hold — a `random()` call is
    /// `VOLATILE` and `pg_column_size` `STABLE`, each refused `42P17` in an index
    /// expression or predicate as on 18.6 — so the round trip stores them at the `Check`
    /// and `ColumnDefault` sites only.
    const NOT_INDEXABLE_CORPUS: &[&str] = &["a < random()", "pg_column_size(s) < 100"];

    const ROUND_TRIP_CORPUS: &[&str] = &[
        // Already-covered shapes, kept so a regression in them fails here too.
        "a > 0",
        "(a) IS NULL",
        "NOT (a > 0)",
        "(- a) < 0",
        "(+ a) < 0",
        "a IN (1, 2, 3)",
        "a NOT IN (1, 2)",
        "COALESCE(a, 0) > 0",
        // The N-ary `AND`/`OR` chain (`render_bool_chain`), which the deparser renders
        // flat where the stored `Expr` is a left-deep nest. Three conjuncts, because
        // the **two**-operand arity is the one a nest and a flat list agree on — the
        // arity every fixture pinning this renderer used before it flattened. Both
        // operators and one mixed nesting, whose inner `OR` is an ordinary argument of
        // the outer `AND` rather than a chain member.
        "a > 0 AND a < 10 AND a <> 5",
        "a > 0 OR a > 1 OR a > 2",
        "a > 0 AND (a < 10 OR a > 100)",
        // A **qualified** `Expr::Column` (`table: Some`) — a different render arm
        // from the bare one, and the qualifier has to survive the round trip.
        "t.a > 0",
        // `Literal::Null`. Riding inside COALESCE keeps the whole expression boolean
        // (a bare `NULL` operand is fine too, but this also pins that the literal is
        // not folded away on the way into the catalog).
        "COALESCE(a, NULL) > 0",
        "lower(s) <> ''",
        "pg_column_size(s) < 100",
        "n = 1.50",
        "(a)::text <> ''",
        "(a)::varchar(3) <> ''",
        "(n)::numeric(8,3) > 0",
        // The **array** spellings of those two, which are a different render arm:
        // the modifier is the element's, so it goes inside the `[]`
        // (`deparse::cast_type_spelling`). Rendering it after the whole name emitted
        // `varchar[](3)`, which re-parses on neither engine — a dump that never
        // reloaded, and this test was green through it because array-ness was not
        // part of `expr_node`'s discriminant. A constant operand rather than a
        // column because `ROUND_TRIP_COLUMNS` has no array column and a scalar
        // column cast to an array type is `42846`.
        "'{ab}'::varchar(3)[] IS NOT NULL",
        "'{1.25}'::numeric(4,1)[] IS NOT NULL",
        // The **sub-second precision** modifier, scalar and array. A string-literal
        // operand rather than the `ts` column so the array line has an operand of the
        // right type (`ROUND_TRIP_COLUMNS` has no array column), and the scalar line
        // matches it. The two types are chosen to cover the **two parser arms** that
        // read a precision — `interval`'s own, and `timestamp_or_time_name`'s, which
        // also has to place the modifier after a `WITH`/`WITHOUT TIME ZONE` clause.
        "'1.23456 secs'::interval(3) IS NOT NULL",
        "'{2020-01-02 03:04:05.6789}'::timestamp(3)[] IS NOT NULL",
        // All four `render_case` arms: searched vs simple (the `operand`) crossed with
        // ELSE vs none, since the two are emitted independently. Without the
        // ELSE-less pair, a renderer that dropped the `ELSE` clause outright would
        // still round-trip every corpus entry that has one.
        "CASE WHEN a > 0 THEN TRUE ELSE FALSE END",
        "CASE a WHEN 1 THEN TRUE ELSE FALSE END",
        "CASE WHEN a > 0 THEN TRUE END",
        "CASE a WHEN 1 THEN TRUE END",
        // Every `BoolTest` spelling: the test word is the render arm, so a renderer
        // that emitted `IS TRUE` for all six would round-trip any single line.
        "(a > 0) IS TRUE",
        "(a > 0) IS NOT TRUE",
        "(a > 0) IS FALSE",
        "(a > 0) IS NOT FALSE",
        "(a > 0) IS UNKNOWN",
        "(a > 0) IS NOT UNKNOWN",
        "s LIKE 'x%'",
        "starts_with(s, 'x')",
        "to_char(ts, 'YYYY') <> ''",
        // MathFn — every member.
        "abs(a) < 10",
        "ceil(n) < 10",
        "floor(n) < 10",
        "round(n) < 10",
        "round(n, 2) < 10",
        "trunc(n) < 10",
        "trunc(n, 1) < 10",
        "sqrt(n) < 10",
        "sign(n) < 10",
        // StringFn — every member, including both spellings that fold onto one
        // variant (`trim` → btrim, `substring … FROM … FOR` → the comma form).
        "upper(s) <> ''",
        "length(s) > 0",
        "replace(s, 'a', 'b') <> ''",
        "substring(s, 1, 2) <> ''",
        "substring(s from 2 for 3) <> ''",
        "ltrim(s) <> ''",
        "ltrim(s, 'x') <> ''",
        "rtrim(s, 'x') <> ''",
        "btrim(s) <> ''",
        "trim(s) <> ''",
        "strpos(s, 'x') > 0",
        "lpad(s, 5) <> ''",
        "rpad(s, 5, '.') <> ''",
        "split_part(s, ',', 2) <> ''",
        "date_trunc('day', ts) IS NOT NULL",
        "isfinite(ts)",
        // The `StringFn` member that reads the **session** rather than the row. The two
        // that read the *catalog* are absent by construction — `pg_get_constraintdef`
        // and `::regclass` are `NOT_STORABLE_STRING_FUNCS` above, so no corpus entry can
        // carry them, and their render arms are proven by `deparse`'s
        // `catalog_reading_string_fns_render_to_their_written_form` instead.
        "current_setting('search_path') <> ''",
        // Always NULL (no column carries a comment here), and storable: it reads no
        // catalog.
        "col_description(a, 1) IS NULL",
        // 18.6's `quote_identifier` rule, and a privilege check pglet's one role passes.
        "quote_ident(s) <> ''",
        "md5(s) <> ''",
        "left(s, 2) <> ''",
        "right(s, -1) <> ''",
        "substr(s, 2) <> ''",
        "substr(s, 2, 3) <> ''",
        "octet_length(s) > 0",
        "mod(a, 3) = 1",
        "power(a, 2) < 100",
        "pow(n, 2) < 100",
        "has_schema_privilege('public', 'USAGE') OR a > 0",
        // The narrowed regex operators: an anchored literal prefix is the only pattern
        // they answer, so it is the only one a stored expression can hold.
        "s ~ '^x'",
        "s !~ '^x'",
        // `POSITION(needle IN haystack)` is stored arg-swapped as `[haystack,
        // needle]`, so a renderer that reads the arguments in order emits the
        // operands backwards. The asymmetric literals make that a *different* stored
        // `Expr`, not merely different text — the symmetric `position(s in s)` would
        // pass a backwards renderer.
        "position('x' in s) > 0",
        "position(s in 'xyz') > 0",
        // VariadicFn — every member.
        "concat(s, 'x') <> ''",
        "concat_ws('-', s, 'x') <> ''",
        "greatest(a, 1) > 0",
        "least(a, 1) > 0",
        "nullif(a, 0) IS NOT NULL",
        // JsonbBuildObject.
        "jsonb_build_object('k', s) IS NOT NULL",
        // Builtin — the clock, random and parenless value-function members, including
        // the four `now`-family calls that each render under their own spelling.
        "ts < now()",
        "ts < transaction_timestamp()",
        "ts < statement_timestamp()",
        "ts < clock_timestamp()",
        "ts <> CURRENT_TIMESTAMP",
        "u <> gen_random_uuid()",
        "a < random()",
        "(ts)::date <> CURRENT_DATE",
        "(ts)::time <> CURRENT_TIME",
        "(ts)::time <> LOCALTIME",
        "ts <> LOCALTIMESTAMP",
        // The nine session-information forms. Each renders to its **written**
        // spelling and must re-parse to the same variant: the six parenless keywords
        // to their bare upper-case words, and `current_schema()` to the *quoted*
        // `"current_schema"()`, which is the one entry whose quoting is load-bearing
        // — an unquoted `current_schema()` re-lexes as the keyword and would restore
        // as `BuiltinFn::CurrentSchema`, a different variant with a different
        // rendering. `IS NOT NULL` rather than a comparison against `s` because the
        // corpus is about the render/re-parse identity, not about `name`'s operators.
        "CURRENT_USER IS NOT NULL",
        "SESSION_USER IS NOT NULL",
        "USER IS NOT NULL",
        "CURRENT_ROLE IS NOT NULL",
        "CURRENT_CATALOG IS NOT NULL",
        "current_database() IS NOT NULL",
        "CURRENT_SCHEMA IS NOT NULL",
        "current_schema() IS NOT NULL",
        "version() IS NOT NULL",
        // ...and the two argument-taking session calls, which are `StringFn`s.
        "current_setting('search_path') IS NOT NULL",
        "current_setting('search_path', TRUE) IS NOT NULL",
        "current_schemas(TRUE) IS NOT NULL",
        // A cast to an enum over a non-constant operand — `StringFunc::EnumIn` under the
        // `text` cast it is planned as, rendered as the one written cast `(m)::rt_mood`.
        // Its operand is the enum's own column: a conversion of any other operand is
        // refused at an index site (`42P17`, `enum_in` not being immutable), and a relabel
        // is stored at all four. At the `DEFAULT` site the operand closes over to the
        // member `'x'::rt_mood`, the plain `text` cast a member literal keeps.
        "(m)::rt_mood IS NOT NULL",
        // Extract — the `retnumeric` split (EXTRACT vs date_part is a *return type*
        // difference, not a spelling one), the bare-word field that folds to a
        // literal, a field literal needing an `E'…'` escape, and the non-literal
        // field that takes the documented pglet-only fallback spelling.
        "extract(year from ts) > 0",
        "extract('dow' from ts) > 0",
        "extract('a\\b' from ts) > 0",
        "date_part('year', ts) > 0",
        "extract(lower('YEAR'), ts) > 0",
        // AnyAll — both quantifiers and a non-`=` operator, plus the `IN` list above
        // (Postgres deparses the two to identical text; pglet must not conflate them).
        "a = ANY (ARRAY[1, 2, 3])",
        "a = ALL (ARRAY[1, 2, 3])",
        "a > ANY (ARRAY[1, 2])",
    ];

    #[test]
    fn render_expr_round_trips_to_the_identical_variant() {
        // The property: `parse(render(e)) == e`, at every stored-expression site.
        //
        // Comparing the **`Expr` values** is the point, and a dump → reload → re-dump
        // byte-equality check (what `persisted_unary_plus_dump_round_trips_through_pglet`
        // does) cannot substitute for it: rendering `StringFunc::Position` as
        // `strpos(…)` evaluates identically and would produce byte-identical text on
        // *both* dumps, while restoring as a different variant — silently changing
        // which queries the planner's structural partial-index implication check lets
        // the restored index serve. Only re-reading the stored `Expr` catches that.
        //
        // Corpus-driven rather than one assertion per arm so the coverage does not rot
        // as variants are added: a new renderable variant needs a line in
        // `ROUND_TRIP_CORPUS`, not a new test.
        for site in StoredExprSite::ALL {
            for src in ROUND_TRIP_CORPUS {
                if NOT_INDEXABLE_CORPUS.contains(src)
                    && matches!(
                        site,
                        StoredExprSite::IndexKey | StoredExprSite::IndexPredicate
                    )
                {
                    // The refusal itself is pinned below, so the skip cannot hide an
                    // entry that has become indexable.
                    assert_eq!(
                        try_store_index(site, src).map_err(|e| e.sqlstate().to_string()),
                        Err("42P17".to_string()),
                        "{site:?}: `{src}` is listed as not indexable"
                    );
                    continue;
                }
                let stored = site.store(src);
                let rendered =
                    render_expr(&stored).unwrap_or_else(|e| panic!("{site:?} render `{src}`: {e}"));
                let restored = site.store(&rendered);
                assert_eq!(
                    restored, stored,
                    "{site:?}: `{src}` rendered as `{rendered}`, which re-parsed to a \
                     different Expr"
                );
            }
        }
    }

    // =========================================================================
    // Corpus completeness — the guard against `ROUND_TRIP_CORPUS` going stale
    // =========================================================================

    /// This node's **render discriminant** and its direct scalar sub-expressions, in
    /// source order. A self-contained sub-`Plan` is a leaf, matching
    /// `Expr::for_each_child`.
    ///
    /// # The discriminant is not the variant tag
    ///
    /// It is the tag **plus every field that selects a render arm** —
    /// `Extract(date_part)` vs `Extract(EXTRACT)` vs `Extract(extract)`,
    /// `AnyAll(ANY)` vs `AnyAll(ALL)`, and so on. Keying coverage on the bare tag
    /// was a hole with teeth: one corpus entry stood in for *every* arm of its
    /// variant, so dropping the `date_part` line left `render_extract`'s
    /// float8-vs-numeric split — a **return-type** difference — completely unproven
    /// while [`round_trip_corpus_covers_every_renderable_form`] still passed. Same
    /// for a renderer that always emits `ANY` and never `ALL`. The `Column`
    /// qualified/bare split was already carved out by hand; folding it in is the
    /// general form of that one-off.
    ///
    /// # Exhaustive on two axes, on purpose
    ///
    /// Every arm matches **without `..`**, so a compile error lands here for a new
    /// `Expr` variant *and* for a new field on an existing one. That is what turns
    /// "does this field select a render arm?" into a forced decision rather than a
    /// remembered one: a field that does not select one is spelled `name: _`
    /// deliberately, next to the fields that do. [`render_expr`] is exhaustive for
    /// the variant axis for the same reason, but nothing there forces the field
    /// axis.
    ///
    /// Where the arm selection is more than a flag test, the discriminant reads it
    /// from the **same** function the renderer dispatches on ([`extract_spelling`],
    /// [`binary_op_sql`]) so the two cannot drift.
    fn expr_node(expr: &Expr) -> (&'static str, Vec<&Expr>) {
        match expr {
            Expr::Literal(_) => ("Literal", Vec::new()),
            // Two render arms: `t.a` and `a`.
            Expr::Column { table, name: _ } => (
                if table.is_some() {
                    "Column(qualified)"
                } else {
                    "Column"
                },
                Vec::new(),
            ),
            Expr::Default => ("Default", Vec::new()),
            Expr::Parameter { index: _ } => ("Parameter", Vec::new()),
            Expr::Bound(_) => ("Bound", Vec::new()),
            // The per-`BuiltinFn` spellings are covered by `BUILTIN_FNS`.
            Expr::Builtin(_) => ("Builtin", Vec::new()),
            Expr::Subquery { plan: _ } => ("Subquery", Vec::new()),
            Expr::Exists {
                plan: _,
                negated: _,
            } => ("Exists", Vec::new()),
            Expr::Negate(e) => ("Negate", vec![e.as_ref()]),
            Expr::UnaryPlus(e) => ("UnaryPlus", vec![e.as_ref()]),
            Expr::Not(e) => ("Not", vec![e.as_ref()]),
            Expr::IsNull(e) => ("IsNull", vec![e.as_ref()]),
            Expr::IsNotNull(e) => ("IsNotNull", vec![e.as_ref()]),
            // Six render arms — `test` selects the spelling, so it is part of the
            // discriminant and each one needs its own corpus line.
            Expr::BooleanTest { expr, test } => (test.sql(), vec![expr.as_ref()]),
            Expr::Lower(e) => ("Lower", vec![e.as_ref()]),
            Expr::ColumnSize { arg, origin: _ } => ("ColumnSize", vec![arg.as_ref()]),
            Expr::Unnest(e) => ("Unnest", vec![e.as_ref()]),
            // `target_type` picks a *name* (`type_oid_sql`), not an arm.
            Expr::Cast {
                expr,
                target_type: _,
            } => ("Cast", vec![expr.as_ref()]),
            // Six render arms: the `(p,s)`, `(n)` and `(p)` modifier spellings, each
            // crossed with **where the modifier goes** — after the name for a scalar
            // target, inside the `[]` for an array one, since the modifier is the
            // element's. `type_oid` therefore selects an arm here where `Cast`'s
            // `target_type` only picks a name, and the predicate is read from
            // `array_element`, which is what `deparse::cast_type_spelling`
            // dispatches on. Keying on `typmod` alone was the hole that let
            // `varchar[](3)` — valid on no engine — ship green.
            //
            // **What this discriminant still cannot express** is the *third*
            // position, because it is [`RenderMode::Catalog`]'s alone: the modifier
            // goes inside the words of `timestamp(3) without time zone` and after the
            // whole name of `character varying(3)`. This walk renders in
            // `Portable`, where every modifier-carrying name is one word, so a
            // `Precision` corpus entry proves the portable spelling and nothing about
            // the catalog one — `deparse`'s
            // `a_catalog_modifier_goes_where_18_6_puts_it` is the other half.
            Expr::CastTypmod {
                expr,
                type_oid,
                typmod,
            } => (
                match (type_oid.array_element().is_some(), typmod) {
                    (false, Typmod::Numeric { .. }) => "CastTypmod(numeric)",
                    (false, Typmod::Length(_)) => "CastTypmod(length)",
                    (false, Typmod::Precision(_)) => "CastTypmod(precision)",
                    (true, Typmod::Numeric { .. }) => "CastTypmod(numeric[])",
                    (true, Typmod::Length(_)) => "CastTypmod(length[])",
                    (true, Typmod::Precision(_)) => "CastTypmod(precision[])",
                },
                vec![expr.as_ref()],
            ),
            Expr::Alias { expr, alias: _ } => ("Alias", vec![expr.as_ref()]),
            Expr::Aggregate {
                func: _,
                arg,
                distinct: _,
                certainty: _,
            } => ("Aggregate", vec![arg.as_ref()]),
            Expr::InSubquery {
                expr,
                plan: _,
                negated: _,
            } => ("InSubquery", vec![expr.as_ref()]),
            // Three render arms: the **N-ary** `AND`/`OR` chain (`render_bool_chain`,
            // which `render_binary_op` takes before anything else), infix
            // (`(l op r)`), and the function form (`starts_with` / `to_char`),
            // selected by `binary_op_sql`. The chain was a render arm with no
            // discriminant of its own, so no corpus entry was required for it.
            Expr::BinaryOp { left, op, right } => (
                if matches!(op.connective(), BinaryOp::And | BinaryOp::Or) {
                    "BinaryOp(bool chain)"
                } else if binary_op_sql(*op).is_some() {
                    "BinaryOp(infix)"
                } else {
                    "BinaryOp(function)"
                },
                vec![left.as_ref(), right.as_ref()],
            ),
            // Two render arms: the `ANY` and `ALL` quantifiers.
            // `sub_select` selects no arm: its right operand, a scalar subquery, declines.
            Expr::AnyAll {
                left,
                op: _,
                right,
                all,
                sub_select: _,
            } => (
                if *all { "AnyAll(ALL)" } else { "AnyAll(ANY)" },
                vec![left.as_ref(), right.as_ref()],
            ),
            // Three render arms, read off the renderer's own selector. Field first,
            // matching `Expr::for_each_child` (source order for both spellings).
            Expr::Extract {
                source,
                field,
                retnumeric,
            } => (
                match extract_spelling(field, *retnumeric) {
                    ExtractSpelling::DatePart => "Extract(date_part)",
                    ExtractSpelling::ExtractFrom(_) => "Extract(EXTRACT)",
                    ExtractSpelling::ExtractComma => "Extract(extract)",
                },
                vec![field.as_ref(), source.as_ref()],
            ),
            // Two render arms: `IN` and `NOT IN`.
            Expr::InList {
                expr,
                list,
                negated,
            } => (
                if *negated { "InList(NOT)" } else { "InList" },
                std::iter::once(expr.as_ref()).chain(list).collect(),
            ),
            Expr::Array(items) => ("Array", items.iter().collect()),
            Expr::JsonbBuildObject(items) => ("JsonbBuildObject", items.iter().collect()),
            Expr::Coalesce(items) => ("Coalesce", items.iter().collect()),
            // The per-member spellings are covered by the `*_FUNCS` arrays.
            Expr::MathFn { func: _, args } => ("MathFn", args.iter().collect()),
            Expr::StringFn { func: _, args } => ("StringFn", args.iter().collect()),
            Expr::VariadicFn { func: _, args } => ("VariadicFn", args.iter().collect()),
            Expr::SequenceFn {
                func: _,
                seq: _,
                args,
            } => ("SequenceFn", args.iter().collect()),
            // Four render arms: `render_case` emits the operand and the `ELSE` each
            // conditionally, so simple-vs-searched and ELSE-vs-none are independent.
            Expr::Case {
                operand,
                when_clauses,
                else_result,
            } => {
                let mut kids: Vec<&Expr> = operand.iter().map(AsRef::as_ref).collect();
                for (cond, result) in when_clauses {
                    kids.push(cond);
                    kids.push(result);
                }
                kids.extend(else_result.iter().map(AsRef::as_ref));
                let tag = match (operand.is_some(), else_result.is_some()) {
                    (false, false) => "Case(searched)",
                    (false, true) => "Case(searched+ELSE)",
                    (true, false) => "Case(simple)",
                    (true, true) => "Case(simple+ELSE)",
                };
                (tag, kids)
            }
            Expr::Window {
                func: _,
                partition_by,
                order_by,
            } => (
                "Window",
                partition_by
                    .iter()
                    .chain(order_by.iter().map(|k| &k.expr))
                    .collect(),
            ),
        }
    }

    /// The [`Literal`] variants a stored expression can hold.
    ///
    /// `Float` is deliberately **absent from [`REQUIRED_LITERAL_TAGS`]**, not from
    /// here: the **parser** constructs none (see `deparse`'s `render_literal`), so no
    /// SQL text
    /// can put one in this corpus — which is the whole claim, since the planner's own
    /// `#[cfg(test)]` serialization corpus does build one. Requiring it would be a permanently red
    /// test; leaving it unlisted with no comment would be the silent gap this block
    /// exists to prevent.
    fn literal_tag(lit: &Literal) -> &'static str {
        match lit {
            Literal::Null => "Null",
            Literal::Bool(_) => "Bool",
            Literal::Int(_) => "Int",
            Literal::Float(_) => "Float",
            Literal::String(_) => "String",
            Literal::Numeric(_) => "Numeric",
        }
    }

    /// Every discriminant [`render_expr`] actually renders — one per **render arm**,
    /// not per variant; see [`expr_node`]. The declined variants are absent because a
    /// *stored* expression can never hold one — `render_expr`'s decline block argues
    /// each case — so no SQL could put one in the corpus; the test asserts that
    /// direction too.
    ///
    /// Hand-kept, and therefore the weak link — see
    /// [`round_trip_corpus_covers_every_renderable_form`] for exactly which steps the
    /// compiler forces and which one it does not.
    ///
    /// **`SequenceFn` is the live instance of that weak link, and it is missing from
    /// this list while [`expr_node`] emits the tag.** `render_expr` renders a
    /// `nextval`/`currval`/`setval` node — it is not among the ten it declines — so
    /// `parse(render(e)) == e` is **unproven for that arm**: nothing here requires a
    /// corpus entry, and the reverse assertion below would read a stored one as
    /// "declines as un-persistable", which is no longer true of it.
    ///
    /// It is not a one-line add, which is why it is recorded rather than closed. The
    /// coverage walk stores every entry at the `Check` site, and a `nextval` in a
    /// `CHECK` is `0A000` here (`nextval() is only supported as a top-level SELECT
    /// expression`) where 18.6 accepts it — so the entry has to be stored at
    /// [`StoredExprSite::ColumnDefault`] instead, which `DEFAULT (nextval('s') > 0)`
    /// does reach (measured, accepted). That means a second, site-specific corpus or a
    /// per-entry site tag, plus the sequence its `@setup` needs; the round-trip is the
    /// interesting half, since the two `nextval` spellings deliberately differ by arm
    /// (see [`render_default`]).
    const RENDERABLE_EXPR_TAGS: [&str; 44] = [
        "Literal",
        "Column",
        "Column(qualified)",
        "BinaryOp(bool chain)",
        "BinaryOp(infix)",
        "BinaryOp(function)",
        "Negate",
        "UnaryPlus",
        "Not",
        "IsNull",
        "IsNotNull",
        // The six `BoolTest` spellings, each its own render arm.
        "IS TRUE",
        "IS NOT TRUE",
        "IS FALSE",
        "IS NOT FALSE",
        "IS UNKNOWN",
        "IS NOT UNKNOWN",
        "Cast",
        "CastTypmod(numeric)",
        "CastTypmod(length)",
        "CastTypmod(precision)",
        "CastTypmod(numeric[])",
        "CastTypmod(length[])",
        "CastTypmod(precision[])",
        "InList",
        "InList(NOT)",
        "Builtin",
        "Array",
        "JsonbBuildObject",
        "AnyAll(ANY)",
        "AnyAll(ALL)",
        "Lower",
        "ColumnSize",
        "Coalesce",
        "Case(searched)",
        "Case(searched+ELSE)",
        "Case(simple)",
        "Case(simple+ELSE)",
        "MathFn",
        "StringFn",
        "VariadicFn",
        "Extract(date_part)",
        "Extract(EXTRACT)",
        "Extract(extract)",
    ];

    /// The [`Literal`] tags the corpus must reach — every variant except the
    /// `Float` no SQL text can produce; see [`literal_tag`].
    const REQUIRED_LITERAL_TAGS: [&str; 5] = ["Null", "Bool", "Int", "String", "Numeric"];

    /// Append `item` unless it is already recorded.
    fn push_unique<T: PartialEq>(seen: &mut Vec<T>, item: T) {
        if !seen.contains(&item) {
            seen.push(item);
        }
    }

    /// What [`ROUND_TRIP_CORPUS`] was observed to actually store, collected
    /// **structurally** from the catalog's `Expr` rather than by searching the
    /// rendered text. Structural is the point: it proves the *parser produced the
    /// variant* (that `trim(s)` really is a `StringFunc::Btrim`, that
    /// `position('x' in s)` really is `Position`), which searching for a spelling
    /// cannot — and `Position` has no spelling to search for, since `deparse`'s
    /// `string_fn_name` never renders it.
    ///
    /// `exprs` holds [`expr_node`]'s **render discriminants**, so an arm-selecting
    /// field (`Extract`'s `retnumeric`, `AnyAll`'s `all`, …) is part of the key
    /// rather than a hand-added side flag. The `qualified_column` flag this used to
    /// carry was the one-off precursor of that; it is now `Column(qualified)`.
    #[derive(Default)]
    struct Coverage {
        exprs: Vec<&'static str>,
        literals: Vec<&'static str>,
        math: Vec<MathFunc>,
        strings: Vec<StringFunc>,
        variadic: Vec<VariadicFunc>,
        builtins: Vec<BuiltinFn>,
    }

    impl Coverage {
        /// Fold `expr` and every sub-expression into `self`.
        fn record(&mut self, expr: &Expr) {
            let (tag, children) = expr_node(expr);
            push_unique(&mut self.exprs, tag);
            match expr {
                Expr::Literal(lit) => push_unique(&mut self.literals, literal_tag(lit)),
                Expr::MathFn { func, .. } => push_unique(&mut self.math, *func),
                Expr::StringFn { func, .. } => push_unique(&mut self.strings, *func),
                Expr::VariadicFn { func, .. } => push_unique(&mut self.variadic, *func),
                Expr::Builtin(func) => push_unique(&mut self.builtins, *func),
                _ => {}
            }
            for child in children {
                self.record(child);
            }
        }
    }

    #[test]
    fn round_trip_corpus_covers_every_renderable_form() {
        // `render_expr_round_trips_to_the_identical_variant` proves the property for
        // whatever the corpus happens to contain — which is worth nothing for a form
        // the corpus does not contain. Nothing else forces the list to grow: the
        // exhaustive `match` forces a *render arm*, the `const fn` name tables force
        // a *name arm*, and neither forces a *corpus line*. That is the tag-set-rot
        // shape the `@profile-lock` guard had to solve with its own completeness
        // check, so the corpus gets one too.
        //
        // What the guard actually forces, measured by adding a throwaway `MathFunc`
        // member and following the errors:
        //
        //  1. the enum gains a member — compile errors in `pglet_sql`'s exhaustive
        //     matches, then in `math_fn_name`;
        //  2. the name arm and the `MATH_FUNCS` entry get added — compile error on
        //     the array's *fixed length*, which has to be bumped deliberately;
        //  3. length bumped, no corpus line — this test, red, naming the member.
        //
        // Coverage is keyed on `expr_node`'s **render discriminant**, not the variant
        // tag. Keying on the tag was a second hole, measured the same way: one corpus
        // entry stood in for every arm of its variant, so deleting the `date_part`
        // line and injecting "render `date_part(f,s)` as `EXTRACT(f FROM s)`" — a
        // *return-type* change, float8 → numeric — left this test green, as did
        // "`AnyAll` always renders ANY". Folding the arm-selecting fields into the key
        // closes it, and `expr_node`'s no-`..` destructuring makes the *next*
        // arm-selecting field a compile error there rather than something to remember.
        //
        // Two holes remain, both hand-kept lists the compiler cannot enumerate:
        //
        //  - **Member arrays.** Step 2's `MATH_FUNCS` entry: a name arm added with the
        //    array left alone compiles and leaves this test green. Rust cannot
        //    enumerate an enum's variants without a derive, so the mitigation is
        //    placement — every member array sits directly beneath the match that
        //    errored. `RENDERABLE_EXPR_TAGS` carries the same hole against
        //    `expr_node`, and `REQUIRED_LITERAL_TAGS` against `literal_tag`.
        //    `BUILTIN_FNS` is the one exception: `builtin_fns_lists_every_variant`
        //    holds it against a wildcard-free match, so a new `BuiltinFn` is a compile
        //    error rather than a silently smaller corpus. The others are unguarded.
        //  - **Per-`BinaryOp` spellings.** The discriminant splits `BinaryOp` where
        //    `render_binary_op` branches — bool chain, infix, function-form — and no
        //    finer, so the *operator* within an arm is unkeyed: re-derive the
        //    spellings from `binary_op_sql` (31 there today; `AND` and `OR` are the
        //    chain arm's, the rest the infix arm's), which has no member array, so a
        //    wrong one (`>=` emitted as `>`) is caught only for the operators the
        //    corpus happens to use. Closing it means a `BINARY_OPS` array plus a
        //    corpus line per operator.
        //
        // Every form is required at the `Check` site alone: the corpus is uniformly
        // boolean-valued so every entry stores at all four sites but the
        // `NOT_INDEXABLE_CORPUS` ones, which store at the other two — the same tree at
        // the `Check` and the two index sites, and the same node shapes over
        // `close_over_columns`' substituted constants at `ColumnDefault` — and the
        // round-trip test above already runs all four.
        let mut seen = Coverage::default();
        for src in ROUND_TRIP_CORPUS {
            seen.record(&StoredExprSite::Check.store(src));
        }

        for tag in RENDERABLE_EXPR_TAGS {
            assert!(
                seen.exprs.contains(&tag),
                "no ROUND_TRIP_CORPUS entry stores an `{tag}`, so the round-trip \
                 property is unproven for the arm that renders it"
            );
        }
        for tag in REQUIRED_LITERAL_TAGS {
            assert!(
                seen.literals.contains(&tag),
                "no ROUND_TRIP_CORPUS entry stores a `Literal::{tag}`"
            );
        }
        for func in MATH_FUNCS {
            assert!(
                seen.math.contains(&func),
                "no ROUND_TRIP_CORPUS entry stores `MathFunc::{func:?}`"
            );
        }
        for func in STRING_FUNCS {
            if NOT_STORABLE_STRING_FUNCS.contains(&func) {
                continue;
            }
            assert!(
                seen.strings.contains(&func),
                "no ROUND_TRIP_CORPUS entry stores `StringFunc::{func:?}`"
            );
        }
        for func in VARIADIC_FUNCS {
            assert!(
                seen.variadic.contains(&func),
                "no ROUND_TRIP_CORPUS entry stores `VariadicFunc::{func:?}`"
            );
        }
        for func in BUILTIN_FNS {
            assert!(
                seen.builtins.contains(&func),
                "no ROUND_TRIP_CORPUS entry stores `BuiltinFn::{func:?}`"
            );
        }

        // The other direction, free from the same walk: `render_expr` declines ten
        // variants on the argument that DDL can never persist one. A corpus entry
        // that stored one would falsify that argument — and would mean `dump_sql` is
        // reachable for a shape it refuses to render.
        for tag in &seen.exprs {
            assert!(
                RENDERABLE_EXPR_TAGS.contains(tag),
                "a ROUND_TRIP_CORPUS entry stored `{tag}`, which `render_expr` \
                 declines as un-persistable"
            );
        }
    }

    /// [`BUILTIN_FNS`] lists **every** [`BuiltinFn`] — closing, for this one family,
    /// the member-array hole the test above documents.
    ///
    /// The hole is that the coverage check walks the *array*, so a variant missing
    /// from it is required by nothing: its `builtin_sql` arm ships with no corpus
    /// entry behind it, and the round-trip property is unproven exactly where a new
    /// spelling was just added. Nothing in the build says so — the array's length is
    /// its own declaration, and `builtin_sql`'s match is exhaustive over the enum
    /// rather than over the array.
    ///
    /// The wildcard-free match at the end is the force: a variant added to
    /// `BuiltinFn` is a **compile error here**, which is what points the author at
    /// both lists. The `contains` walk above it is the runtime half — it goes red when
    /// a member is deleted from the array, which no compiler can see.
    ///
    /// Its siblings (`MATH_FUNCS`, `STRING_FUNCS`, `VARIADIC_FUNCS`,
    /// `RENDERABLE_EXPR_TAGS`, `REQUIRED_LITERAL_TAGS`) carry the same hole and no
    /// such guard; this one is pinned because the `now` family just grew through it.
    #[test]
    fn builtin_fns_lists_every_variant() {
        // Hand-enumerated because Rust cannot iterate an enum's variants; the match
        // below is what makes the compiler insist this list is revisited.
        let every = [
            BuiltinFn::Now,
            BuiltinFn::RandomUuid,
            BuiltinFn::CurrentDate,
            BuiltinFn::CurrentTime,
            BuiltinFn::LocalTime,
            BuiltinFn::LocalTimestamp,
            BuiltinFn::TransactionTimestamp,
            BuiltinFn::StatementTimestamp,
            BuiltinFn::ClockTimestamp,
            BuiltinFn::CurrentTimestamp,
            BuiltinFn::CurrentUser,
            BuiltinFn::SessionUser,
            BuiltinFn::User,
            BuiltinFn::CurrentRole,
            BuiltinFn::CurrentCatalog,
            BuiltinFn::CurrentDatabase,
            BuiltinFn::CurrentSchema,
            BuiltinFn::CurrentSchemaFn,
            BuiltinFn::Version,
            BuiltinFn::Random,
        ];
        for func in every {
            assert!(
                BUILTIN_FNS.contains(&func),
                "`BUILTIN_FNS` does not list `BuiltinFn::{func:?}`, so no \
                 ROUND_TRIP_CORPUS entry is required to store it and its \
                 `builtin_sql` spelling is never round-tripped"
            );
        }
        assert_eq!(
            BUILTIN_FNS.len(),
            every.len(),
            "`BUILTIN_FNS` and this enumeration disagree on the variant count"
        );

        // Exhaustiveness: no wildcard, so a new `BuiltinFn` variant breaks the build
        // here and forces it into `BUILTIN_FNS`, into the list above, and into
        // `ROUND_TRIP_CORPUS`.
        let all_arms_are_covered = |func: BuiltinFn| match func {
            BuiltinFn::Now
            | BuiltinFn::RandomUuid
            | BuiltinFn::CurrentDate
            | BuiltinFn::CurrentTime
            | BuiltinFn::LocalTime
            | BuiltinFn::LocalTimestamp
            | BuiltinFn::TransactionTimestamp
            | BuiltinFn::StatementTimestamp
            | BuiltinFn::ClockTimestamp
            | BuiltinFn::CurrentTimestamp
            | BuiltinFn::CurrentUser
            | BuiltinFn::SessionUser
            | BuiltinFn::User
            | BuiltinFn::CurrentRole
            | BuiltinFn::CurrentCatalog
            | BuiltinFn::CurrentDatabase
            | BuiltinFn::CurrentSchema
            | BuiltinFn::CurrentSchemaFn
            | BuiltinFn::Version
            | BuiltinFn::Random => (),
        };
        all_arms_are_covered(BuiltinFn::Now);
    }

    #[test]
    fn every_function_family_dumps_at_every_stored_expression_site() {
        // The whole-database outage this closes: `render_expr` declined every scalar
        // function family by name, and all four call sites `?`-propagate out of one
        // `Result`, so a single `CHECK (abs(a) < 10)` anywhere made `dump_sql` fail
        // for the entire database. One table per family, all in one schema, so the
        // assertion is that the *dump as a whole* succeeds.
        let db = db_with(&[
            "CREATE TABLE m (a INT CHECK (abs(a) < 10))",
            "CREATE TABLE s (v TEXT CHECK (position('x' in v) > 0))",
            "CREATE TABLE v (t TEXT CHECK (concat(t, 'x') <> ''))",
            "CREATE TABLE j (t TEXT CHECK (jsonb_build_object('k', t) IS NOT NULL))",
            "CREATE TABLE b (ts TIMESTAMP CHECK (ts < now()))",
            "CREATE TABLE e (ts TIMESTAMP CHECK (extract(year from ts) > 0))",
            "CREATE TABLE q (a INT CHECK (a = ANY (ARRAY[1, 2])))",
            "CREATE TABLE k (a INT)",
            "CREATE INDEX k_key ON k ((abs(a)))",
            "CREATE INDEX k_pred ON k (a) WHERE sign(a) > 0",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        // Spot-check the spellings whose *choice* is load-bearing (the rest are
        // covered structurally by the round-trip property test above).
        assert!(
            dump.contains("ADD CONSTRAINT s_v_check CHECK ((POSITION('x' IN v) > 0));"),
            "POSITION operand order: {dump}"
        );
        assert!(
            dump.contains("ADD CONSTRAINT b_ts_check CHECK ((ts < now()));"),
            "now() renders as a call: {dump}"
        );
        assert!(
            dump.contains("ADD CONSTRAINT e_ts_check CHECK ((EXTRACT('year' FROM ts) > 0));"),
            "EXTRACT keeps its numeric-returning special form: {dump}"
        );
        assert!(
            dump.contains("ADD CONSTRAINT q_a_check CHECK ((a = ANY (ARRAY[1, 2])));"),
            "ANY quantifier: {dump}"
        );
        assert!(
            dump.contains("CREATE INDEX k_key ON k ((abs(a)));"),
            "index key expression: {dump}"
        );
        assert!(
            dump.contains("CREATE INDEX k_pred ON k (a) WHERE (sign(a) > 0);"),
            "index partial predicate: {dump}"
        );
    }

    #[test]
    fn parenless_value_functions_render_without_parens() {
        // `CURRENT_DATE`/`CURRENT_TIME`/`LOCALTIME`/`LOCALTIMESTAMP` are Postgres SQL
        // *value functions*, not functions: both engines reject `current_date()`
        // (`42601`). A uniform `name()` renderer would emit an unloadable dump for
        // five of the ten `BuiltinFn` members while the other five need the parens.
        let db = db_with(&[
            "CREATE TABLE t (d DATE, tm TIME, ts TIMESTAMP, u UUID)",
            "CREATE INDEX i1 ON t (d) WHERE d <> CURRENT_DATE",
            "CREATE INDEX i2 ON t (tm) WHERE tm <> CURRENT_TIME",
            "CREATE INDEX i3 ON t (tm) WHERE tm <> LOCALTIME",
            "CREATE INDEX i4 ON t (ts) WHERE ts <> LOCALTIMESTAMP",
            "CREATE INDEX i5 ON t (ts) WHERE ts <> now()",
            "CREATE INDEX i6 ON t (u) WHERE u <> gen_random_uuid()",
        ]);
        let dump = dump_sql(db.schema(), db.storage()).unwrap();
        for expected in [
            "WHERE (d <> CURRENT_DATE);",
            "WHERE (tm <> CURRENT_TIME);",
            "WHERE (tm <> LOCALTIME);",
            "WHERE (ts <> LOCALTIMESTAMP);",
            "WHERE (ts <> now());",
            "WHERE (u <> gen_random_uuid());",
        ] {
            assert!(dump.contains(expected), "missing `{expected}` in: {dump}");
        }
    }

    #[test]
    fn stored_expression_sites_reject_leaves_that_cannot_bind() {
        // `DEFAULT`, `$N`, and an unknown column each used to pass DDL and then fail
        // every write — leaving a table that could never be written to and a database
        // that could never be dumped. Postgres rejects all three at DDL time; the
        // SQLSTATEs are 18.6's, at each of the three sites.
        for (expr, sqlstate) in [
            ("a = DEFAULT", "42601"),
            ("a > $1", "42P02"),
            ("bogus_col > 0", "42703"),
        ] {
            for (label, statements) in [
                (
                    "CREATE TABLE inline CHECK",
                    vec![format!("CREATE TABLE t (a INT, CHECK ({expr}))")],
                ),
                (
                    "ALTER TABLE ADD CONSTRAINT",
                    vec![
                        "CREATE TABLE t (a INT)".to_string(),
                        format!("ALTER TABLE t ADD CONSTRAINT c CHECK ({expr})"),
                    ],
                ),
                (
                    "ALTER TABLE ADD COLUMN inline CHECK",
                    vec![
                        "CREATE TABLE t (a INT)".to_string(),
                        format!("ALTER TABLE t ADD COLUMN b INT CHECK ({expr})"),
                    ],
                ),
                (
                    "CREATE INDEX key expression",
                    vec![
                        "CREATE TABLE t (a INT)".to_string(),
                        format!("CREATE INDEX ix ON t (({expr}))"),
                    ],
                ),
                (
                    "CREATE INDEX partial predicate",
                    vec![
                        "CREATE TABLE t (a INT)".to_string(),
                        format!("CREATE INDEX ix ON t (a) WHERE {expr}"),
                    ],
                ),
            ] {
                let mut db = Database::with_btree();
                let mut last = Ok(Vec::new());
                for sql in &statements {
                    last = db.execute(sql);
                    if last.is_err() {
                        break;
                    }
                }
                let err = last
                    .err()
                    .unwrap_or_else(|| panic!("{label}: `{expr}` was accepted"));
                assert_eq!(err.sqlstate(), sqlstate, "{label}: `{expr}` → {err}");
            }
        }
    }

    #[test]
    fn stored_expression_errors_are_reported_in_postgres_source_order() {
        // The binding check above and the operand-type gate are *both* static, so an
        // expression can carry one of each — and then which error surfaces is a
        // question of traversal, not of rules. Postgres answers it during parse
        // analysis: children before parents, left operand before right, first failure
        // wins. Two whole-tree passes cannot reproduce that (whichever ran first would
        // win regardless of position), which is why `executor::ddl` interleaves the two
        // verdicts in one bottom-up walk. Every row below is 18.6's answer, verified
        // live; the `substring` is the discriminator where both sides share a SQLSTATE.
        for (expr, sqlstate, fragment) in [
            // an operand-type error to the LEFT of a binding error wins...
            ("a LIKE 'x' AND bogus > 0", "42883", "integer ~~"),
            ("a LIKE 'x' AND a > $1", "42883", "integer ~~"),
            // ...and a binding error to the left of an operand-type error wins.
            ("bogus > 0 AND a LIKE 'x'", "42703", "bogus"),
            ("a > 0 AND bogus > 0", "42703", "bogus"),
            // Bottom-up, not top-down: the inner `->` is unresolvable and so is the
            // outer `~~` over its inferred `jsonb`, and Postgres reports the inner one.
            ("(a -> 'k') LIKE 'x'", "42883", "integer ->"),
        ] {
            let mut db = Database::with_btree();
            let err = db
                .execute(&format!("CREATE TABLE t (a INT, CHECK ({expr}))"))
                .err()
                .unwrap_or_else(|| panic!("`{expr}` was accepted"));
            assert_eq!(err.sqlstate(), sqlstate, "`{expr}` → {err}");
            assert!(
                err.to_string().contains(fragment),
                "`{expr}` should report the {fragment} failure, got: {err}"
            );
        }
    }

    #[test]
    fn legitimate_stored_expressions_still_bind() {
        // The guard above is a *tightening*, so pin the shapes it must not reject: a
        // bare column, a table-qualified one, and — the shape to suspect first — a
        // column list that is still in flight (`CREATE TABLE`'s inline CHECK naming a
        // later column, `ADD COLUMN`'s inline CHECK naming the column being added).
        let db = db_with(&[
            "CREATE TABLE t (a INT CHECK (b > a), b INT)",
            "CREATE TABLE q (a INT, CHECK (q.a > 0))",
            "CREATE TABLE r (a INT)",
            "ALTER TABLE r ADD COLUMN b INT CHECK (b > 0)",
            "CREATE TABLE w (a INT, b INT)",
            "CREATE INDEX w_key ON w ((a + b))",
            "CREATE INDEX w_pred ON w (a) WHERE b > 0",
        ]);
        // And they are live constraints, not decorations.
        let mut db = db;
        assert!(db.execute("INSERT INTO t VALUES (1, 2)").is_ok());
        assert!(db.execute("INSERT INTO t VALUES (2, 1)").is_err());
        assert!(db.execute("INSERT INTO w VALUES (1, 2)").is_ok());
        assert!(dump_sql(db.schema(), db.storage()).is_ok());
    }

    /// **A database holding any accepted expression default dumps** — the invariant
    /// `executor::ddl`'s `cook_column_default` render probe exists to hold, stated
    /// from the reader's side.
    ///
    /// The cost of it not holding is not one bad `DEFAULT` clause: `dump_sql` renders
    /// every persisted expression through one `Result`, so a default the deparser
    /// declines takes the **whole database** out of the export — every other table's
    /// schema and rows with it, `pglet_server dump` exiting 1. That is why the guard
    /// here is a whole-dump `is_ok()` over a database that also holds an ordinary
    /// table with rows, rather than a per-default assertion: the ordinary table is
    /// what the failure costs.
    ///
    /// The corpus is the two shapes the cook's own steps separate. `interval '1 day'`
    /// is an `Expr::Bound` whose probe **evaluates**, so the coercion step reaches the
    /// render probe; `… * (1/0)` is one whose probe fails with a deferred code, which
    /// is the arm that used to return early — accepted at DDL, and this test then
    /// failed with `cannot render expression … a plan-internal bound value`. The
    /// `accepted` list beside them is the control: without it the assertion holds for
    /// an engine that refuses every expression default.
    #[test]
    fn a_database_with_any_accepted_expression_default_dumps() {
        let mut db = db_with(&["CREATE TABLE keeper (id INT, v TEXT)"]);
        db.execute("INSERT INTO keeper VALUES (1, 'a')").unwrap();
        for refused in [
            "CREATE TABLE d1 (a INT, b INTERVAL DEFAULT interval '1 day')",
            "CREATE TABLE d2 (a INT, b TIMESTAMP DEFAULT now() + interval '1 day' * (1/0))",
        ] {
            assert_eq!(
                db.execute(refused).map(|_| ()).unwrap_err().sqlstate(),
                "0A000",
                "{refused} must be refused at DDL, or it persists an unrenderable default"
            );
        }
        for accepted in [
            "CREATE TABLE a1 (a INT, b INT DEFAULT (1 + 1))",
            "CREATE TABLE a2 (a INT, b TEXT DEFAULT upper('x'))",
            "CREATE TABLE a3 (a INT, b INT DEFAULT CASE WHEN true THEN 1 ELSE 2 END)",
            "CREATE TABLE a4 (a INT, b TIMESTAMP DEFAULT LOCALTIMESTAMP)",
            "CREATE TABLE a5 (a INT, b INT DEFAULT 1/0)",
        ] {
            db.execute(accepted)
                .unwrap_or_else(|e| panic!("{accepted}: {e}"));
        }
        assert!(
            dump_sql(db.schema(), db.storage()).is_ok(),
            "an accepted expression default made the whole database undumpable"
        );
    }

    /// **A database holding any accepted CHECK / index expression dumps** — the twin
    /// of the assertion above, for the other three persisted-expression sites, and
    /// the invariant `executor::ddl`'s `validate_stored_expr_renders` exists to hold.
    ///
    /// The cost is the same and is why the assertion is a whole-dump `is_ok()` over a
    /// database that also holds an ordinary table with rows: `dump_sql` renders every
    /// persisted expression through one `Result`, so one CHECK the deparser declines
    /// takes the **whole database** out of the export. Reproduced on the tree before
    /// the probe existed — `CREATE TABLE ck1 (t TIMESTAMPTZ, CHECK (t > now() -
    /// interval '1 day'))`, accepted, after which `dump_sql` failed with *cannot
    /// render expression … a plan-internal bound value* and `keeper` and its row went
    /// with it.
    ///
    /// The `refused` list spans the five CHECK spellings and both index positions,
    /// because one covering another is a claim about the code rather than about this
    /// test. They are seven *spellings* over four call sites, not seven call sites —
    /// `execute_create_table` serves the column-level and table-level CHECKs from one
    /// line, and `execute_alter_table_add_check` serves `ADD CHECK` and
    /// `ADD CONSTRAINT … CHECK` from another — so the list is wider than the code and
    /// deliberately so. The `accepted` list beside them is the control: without it the
    /// assertion holds for an engine that refuses every stored expression.
    #[test]
    fn a_database_with_any_accepted_stored_expression_dumps() {
        let mut db = db_with(&[
            "CREATE TABLE keeper (id INT, v TEXT)",
            "CREATE TABLE host (t TIMESTAMPTZ, iv INTERVAL, n INT)",
        ]);
        db.execute("INSERT INTO keeper VALUES (1, 'a')").unwrap();
        for refused in [
            // `validate_check_exprs`, all five spellings.
            "CREATE TABLE c1 (t TIMESTAMPTZ, CHECK (t > now() - interval '1 day'))",
            "CREATE TABLE c2 (t TIMESTAMPTZ, CONSTRAINT c2_recent CHECK (t > now() - interval '1 day'))",
            "ALTER TABLE host ADD CHECK (t > now() - interval '1 day')",
            "ALTER TABLE host ADD CONSTRAINT host_recent CHECK (t > now() - interval '1 day')",
            "ALTER TABLE host ADD COLUMN t2 TIMESTAMPTZ CHECK (t2 > now() - interval '1 day')",
            // `validate_index_exprs` — predicate and key expression.
            "CREATE INDEX host_pred ON host (n) WHERE iv > interval '1 day'",
            "CREATE INDEX host_key ON host ((iv + interval '1 day'))",
        ] {
            assert_eq!(
                db.execute(refused).map(|_| ()).unwrap_err().sqlstate(),
                "0A000",
                "{refused} must be refused at DDL, or it persists an unrenderable stored expression"
            );
        }
        for accepted in [
            "CREATE TABLE a1 (a INT, CHECK (abs(a) < 10))",
            "CREATE TABLE a2 (a INT, b TEXT, CHECK (b LIKE 'x%' OR a > 0))",
            "CREATE TABLE a3 (a INT, ts TIMESTAMPTZ, CHECK (ts <= now()))",
            "CREATE INDEX host_upper ON host ((abs(n)))",
            "CREATE INDEX host_live ON host (n) WHERE n > 0",
        ] {
            db.execute(accepted)
                .unwrap_or_else(|e| panic!("{accepted}: {e}"));
        }
        assert!(
            dump_sql(db.schema(), db.storage()).is_ok(),
            "an accepted stored expression made the whole database undumpable"
        );
    }

    /// A catalog that **already holds** an unrenderable stored expression still fails
    /// loud — the residual the DDL gate above does not reach, and the reason the
    /// store magic did not move for it.
    ///
    /// The gate closes the *producers*; it cannot close a store a **pre-gate build
    /// wrote**. The persisted shape is unchanged (an `Expr::Bound` decodes to the
    /// same value and means the same thing), so that change bumped no magic — bumping it
    /// would brick every ordinary pre-gate database to catch this one — and such a
    /// store reopens, reads and writes normally. What it cannot do is dump, and the
    /// property asserted here is that this stays **loud**: an `Err` naming the
    /// expression, never a silently omitted constraint or a guessed spelling. The
    /// catalog is built by hand for the same reason
    /// `a_dangling_sequence_owner_is_not_emitted`'s is: no accepted DDL reaches the
    /// state any more, and a guard whose only seed is a defect stops testing anything
    /// the moment that defect is fixed.
    #[test]
    fn a_legacy_stored_expression_that_cannot_render_fails_the_dump_loudly() {
        use pglet_sql::catalog::CheckConstraint;
        use pglet_sql::planner::Expr;
        use pglet_sql::types::{PgInterval, Value};

        let mut db = db_with(&["CREATE TABLE legacy (iv INTERVAL)"]);
        // What a pre-gate build persisted for `CHECK (iv > interval '1 day')`.
        db.schema_mut()
            .tables
            .get_mut("legacy")
            .expect("seeded")
            .checks
            .push(CheckConstraint {
                name: "legacy_iv_check".to_string(),
                expr: Expr::BinaryOp {
                    left: Box::new(Expr::Column {
                        table: None,
                        name: "iv".to_string(),
                    }),
                    op: BinaryOp::Gt,
                    right: Box::new(Expr::Bound(Value::Interval(PgInterval {
                        months: 0,
                        days: 1,
                        micros: 0,
                    }))),
                },
            });
        let err = dump_sql(db.schema(), db.storage())
            .expect_err("a legacy unrenderable CHECK must not dump silently");
        assert_eq!(err.sqlstate(), "0A000", "{err}");
        assert!(
            err.to_string().contains("bound value"),
            "the failure must name the expression it cannot render, got: {err}"
        );
    }

    /// A catalog that **already holds** a table and an enum of one name fails the dump
    /// loud — the pair the type-namespace gates refuse to build, which a pre-gate store
    /// holds and still opens. Built by hand for the reason the test above gives: no
    /// accepted statement reaches the state any more. The enum is moved under the
    /// table's name from a real `CREATE TYPE`, so its OID and labels are what a pre-gate
    /// build persisted.
    ///
    /// The controls are the pairs that reload, which the refusal must not reach: a table
    /// whose quoted name is not the enum's folded key (`"Ty9"` beside an enum created as
    /// `"Ty9"`, key `ty9`), reloaded here statement by statement, and the same table once
    /// the enum is gone.
    #[test]
    fn a_legacy_table_named_like_an_enum_fails_the_dump_loudly() {
        let mut db = db_with(&[
            "CREATE TABLE ty8 (x INT)",
            "INSERT INTO ty8 VALUES (1), (2)",
            "CREATE TYPE ty8_src AS ENUM ('a')",
        ]);
        let enums = &mut db.schema_mut().enums;
        let moved = enums.remove("ty8_src").expect("seeded");
        enums.insert("ty8".to_string(), moved);
        let err = dump_sql(db.schema(), db.storage())
            .expect_err("a table and an enum of one name must not dump silently");
        assert_eq!(err.sqlstate(), "0A000", "{err}");
        assert!(
            err.to_string()
                .contains("cannot dump table ty8 beside the enum type"),
            "the failure must name the table, got: {err}"
        );
        db.schema_mut().enums.remove("ty8");
        assert!(
            dump_sql(db.schema(), db.storage()).is_ok(),
            "the table alone must dump"
        );

        let quoted = db_with(&[
            "CREATE TYPE \"Ty9\" AS ENUM ('a')",
            "CREATE TABLE \"Ty9\" (x INT, m ty9)",
        ]);
        let dump = dump_sql(quoted.schema(), quoted.storage())
            .expect("a table beside an enum of another catalog key dumps");
        assert!(
            dump.contains("CREATE TYPE ty9 AS ENUM") && dump.contains("CREATE TABLE \"Ty9\""),
            "{dump}"
        );
        let mut restored = Database::with_btree();
        for block in dump.split("\n\n") {
            let sql = block.trim();
            if sql.is_empty() || sql.starts_with("COPY") {
                continue;
            }
            restored
                .execute(sql)
                .unwrap_or_else(|e| panic!("restore failed on `{sql}`: {e}\n{dump}"));
        }
        assert!(restored.schema().enums.contains_key("ty9"));
        assert!(restored.schema().get_table("Ty9").is_some());
    }
}

#[cfg(test)]
mod column_default_round_trip {
    use super::{ColumnDefault, render_default};
    use crate::Database;

    /// The `ColumnDefault` a `CREATE TABLE … DEFAULT <sql>` actually stores.
    ///
    /// The defaulted column is **`TEXT`**, and the type is load-bearing rather than
    /// arbitrary: `executor::ddl`'s `cook_column_default` runs the target type's input
    /// function on an unknown literal at DDL time, as 18.6 does, so `DEFAULT 'x'` into
    /// an `INT` is `22P02` on both engines and cannot be a corpus entry. `text` accepts
    /// every literal spelling below, which is what lets one column carry the whole
    /// corpus — and it changes nothing this test is about, since a `ColumnDefault` is
    /// stored uncoerced (the coercion happens at each write).
    fn stored(sql: &str) -> Option<ColumnDefault> {
        let mut db = Database::with_btree();
        db.execute(&format!(
            "CREATE TABLE t (a TIMESTAMPTZ, b DATE, c TIME, d UUID, e TEXT DEFAULT {sql})"
        ))
        .unwrap_or_else(|e| panic!("DEFAULT {sql}: {e}"));
        db.schema().tables["t"].columns[4].default.clone()
    }

    /// `parse(render(d)) == d` for every renderable `ColumnDefault` — the twin of
    /// `render_expr_round_trips_to_the_identical_variant`, which covers the `Expr` path
    /// (CHECK / index key / index predicate) and **not** this one.
    ///
    /// It exists because `test:dump`'s corpus carries no `now`-family column default,
    /// so nothing there renders these arms. Collapsing them would turn a volatile
    /// `DEFAULT clock_timestamp()` into a stable `now()` across a dump/restore with
    /// every gate green.
    ///
    /// The `NextVal` arm is deliberately absent: it is hoisted out of `CREATE TABLE`
    /// into an `ALTER TABLE … SET DEFAULT` and covered by the dump corpus's own
    /// sequence cases.
    #[test]
    fn every_column_default_renders_back_to_the_same_variant() {
        let corpus = [
            "5",
            "-7",
            "1.5",
            "'x'",
            "true",
            "false",
            // The five `now`-family spellings — each must survive as itself.
            "now()",
            "transaction_timestamp()",
            "statement_timestamp()",
            "clock_timestamp()",
            "CURRENT_TIMESTAMP",
            "CURRENT_DATE",
            "CURRENT_TIME",
            "gen_random_uuid()",
        ];
        let mut seen: Vec<ColumnDefault> = Vec::new();
        for sql in corpus {
            let default = stored(sql).unwrap_or_else(|| panic!("DEFAULT {sql} stored nothing"));
            let rendered = render_default(&default).unwrap_or_else(|e| panic!("{sql}: {e}"));
            let restored =
                stored(&rendered).unwrap_or_else(|| panic!("DEFAULT {rendered} stored nothing"));
            assert_eq!(
                restored, default,
                "`{sql}` rendered as `{rendered}`, which re-parsed to a different default"
            );
            assert!(
                !seen.contains(&default),
                "`{sql}` stores a default some earlier spelling already stored — the \
                 spellings are collapsing rather than round-tripping"
            );
            seen.push(default);
        }
        // `DEFAULT NULL` is no default at all (18.6 stores no `pg_attrdef` row), so it
        // is not in the corpus above and the emitter must write no DEFAULT clause.
        assert_eq!(stored("NULL"), None);
    }
}
