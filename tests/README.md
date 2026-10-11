# pglet Tests

## Fixtures (`fixtures/`)

SQL fixtures compare pglet output against real Postgres. The default `@test`
comparison runs **both** wire formats — extended-protocol binary (validated against
the recorded `.expected.json`) and simple-query text (compared live, pglet-vs-Postgres)
— so a text-encoder-only bug can't pass green just because the binary path is the one
exercised. **The binary leg is only as wide as its decoder.** A column's bytes are turned into
JSON by `pglet_tests`' `column_value_to_json`, whose arm list is derived from `TypeOid` —
the declaration side — rather than from the decoder (`regtype` has an arm for the oracle
side alone, pglet not modelling it). A type with no arm fails the fixture by name rather
than recording JSON `null`, which would make that column's comparison vacuous
(`null == null` whatever either engine sent) and write a false value into the recorded
`.expected.json`, since the same function records the oracle. That keeps a false value out
of the record — but it is a live *comparison* only where a leg reads the
column: an `@expect-error` fixture 18.6 **accepts** records Postgres's rows and compares
none of them, so regenerating one corrects the record and makes nothing live. The
derivation, not the fallback, is what keeps the list whole. The `-- @param` path needs
no arm: the bindable type spellings (`FixtureParamType`) hold none of the three.

**An arm can also be narrower than the value it decodes**, which is the same blindness
one layer in. Two arms format with `[subsecond digits:3]` — `Type::TIMESTAMP` and
`Type::TIMESTAMP_ARRAY`, and they are the only two in the file that fix a subsecond
width at all — so a `timestamp` column's JSON, scalar or array, is milliseconds and the
binary leg cannot see a **sub-millisecond** difference: both sides record `.678` for
`…05.678912`. The **text** leg does see it, being a live full-precision comparison, and
that is measured rather than assumed: a mutation that rounds a `timestamp(6)` at 5
digits fails a fixture `(text format)` and passes the binary one. `Type::TIMESTAMPTZ`
and `Type::TIMESTAMPTZ_ARRAY` have no such gap (both format RFC 3339, full precision),
so the asymmetry is the arm's and not the type's. The `date` and timestamp arms record the
binary `infinity` sentinels as the strings `infinity` / `-infinity`. A `float8`'s arm
records `Infinity` and `-Infinity` as JSON `null`, measured with `date_part` of an infinite date,
so the binary leg cannot tell `Infinity` from NULL; the text leg can.

For a **read-only** `@test` both formats are captured within a single
`BEGIN … ROLLBACK`, so the setup runs once; a **mutating** `@test` re-runs setup in a
separate transaction for the text pass (running it twice in one txn would let the text
pass observe the binary pass's writes). Read-only is detected conservatively from the
statement text — anything uncertain takes the safe two-transaction path.

**What a fixture compares comes from the native server**, which never runs the embed
facade's own statement loop — `Database::execute_as`'s and `execute_bound`'s parse-first
route and `execute_bound`'s plan cache. A fixture run says nothing about those;
`crates/pglet`'s own tests and the wasm and npm suites are their guards.

### Transaction isolation, and what it costs

Each fixture runs inside an isolating `BEGIN … ROLLBACK`, so it leaves no persistent
state and **needs no `DROP TABLE IF EXISTS` boilerplate** — the harness also resets
Postgres's `public` schema once at startup, so a clean slate is guaranteed.

**What that costs, beyond the recorded "the wrapper restores the snapshot":** the harness
cannot express an **aborted transaction block** at all, even with `@no-txn`. It propagates
the first error from every statement it runs — `run_setup` with `?`, `execute_test` by
returning — so the statement that *creates* the aborted state and the statement that
*observes* it cannot sit on opposite sides of the compared boundary: put the failure in
`@setup` and the fixture's compared result *is* that failure, put it in `@test` and nothing
after it runs. Guards for that class are unit tests (`crates/pglet`, `pglet_server`) and
`test_concurrent`, not fixtures.

**A fixture cannot script a second connection either.** The harness *opens* one — the
declared-type text probe replays the setup on its own connection — but sequentially, and
a fixture's directives address only the connection its `@setup`/`@test` run on. So a
divergence that needs two connections interleaved is outside the corpus by construction —
the writer's own costs (an unrelated write waiting for a block, an aborted block holding
the writer until it ends), another connection's read through a block's uncommitted DDL,
and a statement naming an object an open block created are among them (`EDGES.md` rows
228, 230, 221 and 240). Each such row's `EDGES.md` guard cell records what stands behind
it, and the reason it names is that row's own.

```sql
-- @setup
CREATE TABLE users (id INT, name TEXT);

-- @test
SELECT * FROM users;
```

### Commands

```bash
deno task test:fixtures              # Run fixtures
deno task test:fixtures:update       # Regenerate expected from Postgres
deno task test:fixtures select/      # Filter by pattern
deno task test:fixtures:profile-lock # Stripped-build regression lock
```

### Directives

- `-- @setup` - SQL to run before the test (optional, uses simple query for multi-statement)
- `-- @test` - The query to test (required). Compared in **both** binary (vs
  `.expected.json`) and text (live vs Postgres) wire formats, unless `@text`/`@param`
  narrows it to one.
- `-- @expect-error: message` - Test expects an error (optional). pglet must
  error; and if Postgres errors too (the regenerated `.expected.json` records an
  error shape), pglet's **SQLSTATE must match Postgres's**. When Postgres instead
  accepts the query (the deliberately pglet-stricter cases, e.g.
  `select/natural_join_loud.sql` — recorded as a success shape), any pglet
  error passes. The trailing message is a human label, not matched.

  **A `@setup` failure is reported as the fixture's entire result**, and that makes two
  fixture shapes vacuous-but-green. A fixture whose script *deliberately* fails mid-setup —
  to abort a transaction block, say — never reaches its own `@test` at all: the harness
  stops at the failing setup statement, records that error, and compares it. And an
  `@expect-error` fixture whose setup can fail with the **same** SQLSTATE the test asserts
  scores green off the wrong statement, which no mutation of the code under test can show.
  Both shapes have shipped green and been caught only by mutation. The remedies: assert
  the *consequence* as a value fixture (setup runs to completion; a broken engine makes it
  fail or answer differently), keep an error fixture's setup incapable of failing, and put
  what neither shape can reach — an aborted block's own answers — in a unit test instead.
- `-- @text` - compare **only** the text-format wire output (simple query protocol)
  against the recorded `.expected.json`. Normal fixtures already check text live, so
  reach for `@text` when the text output *is* the point and you want it snapshotted —
  e.g. a `timestamptz` rendered with its `+00` offset (`types/timestamptz_text.sql`).
- `-- @param $N <type> <literal>` - bind a parameter to the test query (optional, repeatable)
- `-- @param-declared $N <type> <literal>` - the same, with the parameter's type declared
  in the `Parse` message; see **`-- @param`** below
- `-- @no-txn` - run setup+test **without** the isolating `BEGIN … ROLLBACK` wrapper,
  for fixtures that drive their own transactions (the `transaction/*` suite); these
  keep their own `DROP`-first cleanup since they aren't rolled back.
- `-- @profile-lock` - re-run this fixture against the stripped
  (`--no-default-features`) server in `deno task test:fixtures:profile-lock`. Required
  when the `@test` takes a **different plan** on the two build profiles. See
  **Profile lock** below.
- `-- @no-profile-lock: <why>` - the justified opt-out from the
  `@expect-error`-over-an-index rule below. The justification is **required** — the
  rule's whole job is to force a decision, and an unexplained waiver is the silent
  omission wearing a directive.
- `-- @no-declared-types: <why>` - the justified opt-out from the **declared column
  type** comparison below, on **both** legs. For a fixture whose values match Postgres
  but whose declared types diverge for a reason outside the fixture's subject — a type
  pglet doesn't model, a catalog view's column it declares differently. Justification
  **required**, same contract as `@no-profile-lock:`.
- `-- @no-error-fields: <why>` - the justified opt-out from the **structured error
  field** comparison below. For an `@expect-error` fixture where pglet sends a
  *different* value than 18.6 by design — the clearest case is a `42601` whose parser
  stops at an earlier token, so both engines send a `P` and disagree about which token
  is wrong. Justification **required**, same contract as the other two.
- `-- @expect-message` - also require pglet's primary message (`M`) to equal Postgres's
  **byte for byte**. Opt-in, not default-on: `M` diverges on a long, separately-tracked
  list (`EDGES.md`), so a default-on comparison would redden them all at once and this
  directive would immediately become a waiver worn by half the corpus. Opt-in makes it a
  **lock** — a fixture carrying it asserts that this message is now 18.6's, and reddens
  the day it stops being.

### Directive parsing

**Unknown directives are a hard error.** A comment line whose first non-`-`,
non-whitespace character is `@` followed by `[A-Za-z0-9_]` must name a directive above,
or it fails the fixture's parse, naming the file and the offending line. Silently
absorbing it as inert SQL-comment text is how a typo (`-- @profile-lokc`, or
`-- @expect-error` missing its colon) quietly changes what a fixture asserts while the
run still reports green.

The directives a fixture may name are `parse_fixture`'s arms in
`crates/pglet_tests/src/lib.rs` (and `test_copy`'s `parse_copy_fixture` arms for a `copy/`
fixture); `KNOWN_DIRECTIVES` and `KNOWN_COPY_DIRECTIVES` only word the refusal, and can go
stale against those arms without reddening anything.

Two spellings are normalized so they cannot slip past that check:

- **Spacing between `--` and `@` is not significant.** `-- @test`, `--@test`, and
  `--   @test` are one directive. Requiring the literal `-- @` meant `--@no-txn` parsed
  clean with the flag unset.
- **The identifier class is `[A-Za-z0-9_]`,** not letters only, so `-- @1foo` and
  `-- @_foo` are candidates (and therefore rejected) rather than inert text.

Prose about the containment operator (`-- @> true: …`) is unaffected: `>` is outside
that class, so `@>` is never a candidate.

A malformed fixture is **failed**, not skipped. Only the `copy/` family — which the
`test_copy` binary parses with its own `@copy-from` / `@copy-to` / `@data` /
`@data-hex` / `@verify` grammar — is skipped as foreign, and only for a fixture that
actually lives under `copy/`. `data` and `verify` are ordinary English words; honoring them by name anywhere
meant a stray `-- @verify the ordering is stable` in any fixture turned that fixture
into a silent skip. Skipped here, a `copy/` fixture is parsed by `test_copy`, which reads
a directive line through the same predicate, so a line is a directive line there exactly
when it is one here, and fails a fixture whose directive line names none of its
directives.

**Known parser limits.** The line scan has no quote awareness, so a `-- @foo` line
inside a string literal or a `$$ … $$` block is falsely rejected (no fixture hits this
today; the workaround is to reword or split the literal). Directive detection is also
line-based and unaware of SQL structure, which is what makes the crude rules above
tractable.

**Do not write a trailing `--` comment after the `@test`'s final `;`.** The block is
split on `;` and every non-blank piece is executed, with the **last** one's rows,
row count and declared columns taken as the result. A comment-only tail is not blank,
so it executes, returns nothing, and overwrites all three. It has **two** outcomes, and
the quiet one is the one the ordinary authoring workflow produces:

- **On an existing fixture** — one whose recorded expectation already holds rows — the
  comparison fails and the run reports `expected file is stale (run --update)`. Loud,
  but the message names the wrong repair: an `--update` would record the empty result
  and permanently silence both type legs for that fixture.
- **On a new fixture written with the tail and `--update`d in the same act** there is no
  message at all. `run_update_mode` regenerates through the *same* `execute_test`, so it
  records `{rows: [], rowCount: 0, columns: []}`; both comparisons then match, and the
  declared-type legs bucket the fixture as `no_columns`, which is a legitimate outcome
  they count rather than fail. **Green from birth, and no human error is needed for it**
  — the fixture simply asserts nothing, forever.

Put per-column notes **inside** the statement (as the `select/alias_*_declared_types.sql`
family does) or above it. Corpus scan: 0 of the fixtures are affected — by two
independent re-implementations of `parse_fixture`, neither of which is the harness's own
code, so read that as two agreeing checks and not as a proof.

### Declared column types

Comparing values is not enough, and there is no single
"declared type" to compare: pglet computes its output column types through **two
different walkers**, one per protocol, and each feeds a different client. (This
section is the *harness* half; the walker seam itself — the disagreement
classes, the `LeafScope` unification state — is `docs/declared_types.md`.)

| leg | pglet computes it in | what consumes it |
| --- | --- | --- |
| **text** | the executor's `QueryResult::shape` (`expr_type_oid`) | the simple-query `RowDescription` — psql, and the text rendering in both protocols |
| **extended** | the planner's `Plan::result_shape` (`Expr::infer_type`), via `Describe` | the extended-protocol `RowDescription` — what a **binary** client decodes against |

The harness compares the **declared type OID of every column** on **both** legs, live
pglet-vs-Postgres, and neither leg subsumes the other: the text leg is where a wrong
`RowDescription` hides behind a byte-identical value, the extended leg is where a wrong
but width-compatible type decodes cleanly, and a disagreement **between the two walkers**
is visible only to the pair. Why each of those holds — with the measured instances,
including the alias-scope one `select/alias_*_declared_types.sql` spans — is
`docs/declared_types.md`.

**Both legs are structurally blind to a bare enum result column, and the reason the
count is green is that no fixture has one.** `compare_declared_types` is a plain
`a.type_oid != b.type_oid` with no enum normalisation, while pglet declares an enum
column `25`/`text` and 18.6 declares the enum's own OID with `typlen` `4` — a genuine
wire divergence (an enum value is a `Value::Text` here, `docs/enums.md`). No fixture in
the corpus projects a bare enum column, and the green declared-type counters are the
evidence for that rather than a reading of the 400-odd `enum/` files: one that did would
be **red today**, and would need `@no-declared-types` naming this. Two things the legs do
not compare at all: a column's **type modifier**, and its `table_oid`/`column_id`
provenance — so a divergence in either is invisible to the whole differential set and
has to be asserted in Rust (`crates/pglet/tests/cte_column_list.rs` is the one that does).

The two legs read their OIDs very differently, and only one needed building:

- **extended** — free. A prepared statement's `Describe` reply is exactly what
  `tokio_postgres::Statement::columns` carries, so the harness prepares each `@test`
  statement explicitly (which `client.query(&str, …)` already did internally) and reads
  the columns off the handle. No new wire code, and the columns are there even when the
  query returns **zero rows**.
- **text** — hand-rolled. tokio-postgres discards the simple query's `RowDescription`
  (`SimpleColumn` carries only a name), so `SimpleProbe`
  (`crates/pglet_tests/src/simple_probe.rs`) speaks the protocol directly. It is a
  **separate, additive pass** on its own connection: only the types are read off it,
  never the rows. Being a second connection it replays the fixture's setup, which is
  invisible for a transactional setup and deliberately ignored where it isn't (a
  sequence pglet's `ROLLBACK` doesn't restore would otherwise fail on the replay rather
  than on anything the engine got wrong).

The two legs cover **different corpora**, which is the other half of the extended leg's
value: `@param` fixtures (the simple query protocol cannot bind) and `@no-txn` ones are
extended-only, while `@text` fixtures are text-only. Each leg counts and reports its own
coverage.

The `@no-txn` exclusion on the text leg has a recorded reason — `has_text_leg`'s doc in
`test_fixtures.rs`: that leg's types come from a **second** connection replaying the
setup, which a fixture driving its own transactions cannot be assumed to survive. The
extended leg needs no such exclusion, its types riding the fixture's own binary run.
Whether the exclusion could be lifted is a separate question and is not settled here: an
earlier measurement found the suite green with it removed, under a different
cross-connection rule: a second connection replaying setup beside a `@no-txn` fixture's
open block that has written waits for it — `55P03` past the default `1s` `lock_timeout`.
Re-measure before treating it as removable.

The types are **never recorded** in `.expected.json` on either leg — the comparison is
live, so an `--update` run cannot freeze a divergence into the snapshot.

**The check reports what it reached.** Every run prints, **per leg**, how many fixtures
it actually asserted on versus the ways it had nothing to say, because the failure mode
here is silence: a check that quietly stops checking looks exactly like a check that
passes. Every eligible fixture lands in exactly one bucket, the red ones included — a
total that dropped its mismatches would under-report precisely when the number matters.
The comparison is deliberately four-way rather than "mismatch or not", since two of
those outcomes are *not* assertions and must not be confused with agreement:

- **both sides described the same columns** — the assertion ran and held;
- **neither side described a row** — the `@test`'s last statement emits no
  `RowDescription` (an `INSERT`/DDL without `RETURNING`). Structural, so it needs no
  per-fixture opt-out;
- **the `@test` errored identically on both engines** — a rejected statement describes
  no columns, and the SQLSTATE is asserted by the value comparison;
- **the read failed** — the fixture **fails**. This is the absence of evidence, never
  evidence of agreement, and a harness fault reproducing on *both* connections
  (`CLIENT-FAULT`) is excluded by name so it cannot match itself into a pass. It is the
  same hole `CLIENT_FAULT_CODE` closes on the `@expect-error` arm: an error in the
  checking machinery scoring as the absence of a divergence.

Counts differing between the two sides — including one side describing five columns
against the other's zero — are a **mismatch**, not a non-assertion.

Both legs run before either fails the fixture, so one going red never suppresses the
other's count, and a fixture divergent on both reports both.

Three gaps this check does not close:

- **Duplicate output column names are untestable.** `row_to_json` keys a
  `serde_json::Map` by column name, so a `@test` projecting two columns named `x`
  collapses them to one — in the values *and* in the type list. Give such a fixture
  distinct `AS` labels.
- **Typmod is not compared** — both legs keep name + OID only, so a column can be declared
  the right *type* with the wrong *precision/length* and still pass. One cause is an
  **explicit cast**, whose typmod is applied to the value but reported as `-1` in the
  `RowDescription`, where Postgres reports the encoded modifier. Read off the raw wire,
  `'x'::varchar(7)` · `1::numeric(8,3)` · `'…'::timestamp(3)` · `'…'::time(3)` ·
  `'…'::interval(3)` describe as `11 · 524295 · 3 · 3 · 2147418115` on 18.6 and `-1` at
  every one here. The **population** is whatever the corpus happens to project, so take
  the criterion rather than a count: a fixture reddens when its `@test` projects an
  explicit cast whose type carries a modifier 18.6 encodes into the `RowDescription`,
  which takes in the temporal `(p)` spellings beside the length/scale ones. A `float(n)`
  cast is **not** in it: its modifier is spent selecting the type, so `0.1::float(1)`
  describes as `real` and `0.1::float(30)` as `double precision`, both with `-1`, on 18.6
  and here alike. A base column projected through a plain `SELECT` carries typmod
  correctly on both legs; the same column's typmod — and its provenance — is dropped on
  the `Describe` leg of a `RETURNING` (`EDGES.md` row 11), and is
  `-1` on both legs through a set operation, a `JOIN … USING` merge (its provenance too)
  or a scalar subquery where 18.6 keeps an agreeing one (row 155). A **value constructor**
  joins the cast in the same class, and its cost is a wrong width rather than a missing
  one: 18.6's `exprTypmod` gives a `CASE`/`COALESCE`/`GREATEST` its arms' modifier when
  they agree, so `COALESCE(v, v)` over a `varchar(5)` is `9` there and `-1` here. The
  whole class is `EDGES.md` row 155. The harness side of comparing typmod is small; what
  it is gated on — making a computed column's `RowDescription` carry the modifier — is
  not.

  The one place the value is **decided** rather than defaulted is a recursive CTE's
  rebuilt columns, whose modifier is resolved by `select_common_typmod` over the two
  terms' modifiers as pglet reads them — which keeps the declared width one the CTE's own
  rows satisfy wherever the read is the column's own (`docs/declared_types.md` § A
  recursive CTE's declared modifier). That is invisible here for the same reason
  everything above is, and is pinned by `crates/pglet/tests/cte_column_list.rs`.
- **Neither leg runs for `@expect-error`**, on either walker: pglet rejecting the query
  *is* the assertion, so there is no agreed `RowDescription`, and the
  deliberately-pglet-stricter half (Postgres accepts, pglet rejects) would read as a
  failed read rather than a result. Beyond that each leg has its own exclusions —
  `@param` and `@no-txn` for the text leg, `@text` for the extended one — and
  `@no-declared-types:` waives both.
- **The opt-out is not per-leg, deliberately.** A fixture that could waive one walker
  while the other still passed would hide exactly the pglet-internal split the pair
  exists to expose, so `@no-declared-types:` silences both or neither. The cost is
  real — a waiver written for an executor-walker reason also drops that fixture's
  planner-walker coverage — and is why the per-leg `not eligible` counts are printed
  rather than buried.
- **It checks the corpus, not the dialect.** The gate only sees SQL some fixture
  actually issues. A divergence in a shape no fixture exercises stays invisible however
  sharp the check is, so a green run is a statement about coverage as much as about
  correctness — read the per-leg `Asserted: N of M` line accordingly.

### Structured error fields

An `@expect-error` fixture whose SQLSTATE matches then compares the **structured**
`ErrorResponse` fields both engines sent — detail (`D`), hint (`H`), position (`P`),
schema/table/column/constraint (`s`/`t`/`c`/`n`), read off tokio-postgres's `DbError`
accessors on each side.

**Live, never recorded.** These are captured in the same run and compared pglet-against-
Postgres, exactly as the declared column types are; `QueryError::fields` is
`#[serde(skip)]`, so no `.expected.json` carries them. That is what let the whole
capability land without regenerating a single recorded error — verified: a full
`test:fixtures:update` after the change rewrote **zero** existing files.

**The scoring rule is asymmetric**, and the asymmetry is the design (`docs/errors.md`):

- pglet **omits** a field 18.6 sends → agreement. Omitting what it cannot fill truthfully
  is pglet's declared posture, and scoring it red would make the correct answer
  indistinguishable from a wrong one.
- pglet sends a field 18.6 omits, or sends it with a different value → **failure**. That
  is the invention the comparison exists to catch.

**So it is blind in two directions.** A fixture stays green when pglet *stops* sending a
field it sent before — an omission scores as agreement. And the comparison runs only in
the `(pglet Error, recorded Error)` arm, so the **pglet-stricter** fixtures — an
`@expect-error` whose recorded `.expected.json` holds a *success*, because Postgres
accepted the statement, so any pglet error passes — have no oracle field set to compare
against and an **invention** on one of those is invisible too. Both halves are guarded
outside this harness — `pglet_core`'s
`error_fields` unit tests pin the renderers and the deliberate omissions, `pglet_sql`'s
`engine_constraints` pins that the **executor** actually fills each payload,
`pglet_server`'s `encoding::tests` pins the raw field bytes per send site, and
`tests-web/npm/dx_test.ts` pins the wasm host's property names. Do not read a green
fixture run as "the fields are still being sent."

`-- @expect-message` additionally locks the primary message; `-- @no-error-fields: <why>`
waives the field comparison for a justified divergence.

### Profile lock

pglet builds in two shapes over one core — the default with
`index_acceleration` on, and the stripped `--no-default-features` build whose planner
emits `Filter(Scan)` for every predicate. An index must change only a query's *speed*,
never its *result*, so the differential output has to be identical on both — an intent
with measured exceptions outside the corpus, which `docs/indexes.md` § Build profiles and
`EDGES.md`'s header name. The
lock run enforces it on the fixtures where the two actually diverge:

```bash
deno task test:fixtures:profile-lock   # PGLET_NO_INDEX_ACCEL=1 … -- --profile-lock
```

It builds and spawns the **stripped** server, then runs only the `@profile-lock`
fixtures. Two completeness checks run first, so the tag set can't fall behind as
fixtures are added:

1. **Plan-derived.** Every fixture's `@test` is planned in-process; one that plans to
   an `IndexScan` / `IndexRangeScan` / `GinScan` and lacks the directive fails the run.
   (This check needs the harness binary itself built with acceleration on — its default;
   built `--no-default-features` it cannot derive accelerated plans, so it prints a loud
   note and self-skips rather than pass vacuously.)
   Tagging a fixture that plans no accelerated node is fine and sometimes required: the
   decline-locks (`select/index_probe_type_mismatch_loud.sql` and siblings) assert that
   the accelerated planner *declines* the index and raises the same error the stripped
   build does, which by definition produces no accelerated node to detect.
2. **`@expect-error` over an index.** Check 1 is structurally blind to one class.
   Index selection runs *inside* the planner, so a query that errors **during** it — a
   probe type-mismatch, an operand gate — yields no plan to inspect and is classified
   "not accelerated". That is exactly a shape where the profiles can disagree: the
   accelerated build raises from the probe gate (green against Postgres in the main
   run) while the stripped `Filter(Scan)` may return rows, or a different SQLSTATE, and
   never runs at all. So an `@expect-error` fixture whose `@setup` builds any index
   (`CREATE INDEX`, `UNIQUE`, `PRIMARY KEY`) must carry `-- @profile-lock` or a
   justified `-- @no-profile-lock:`.

**Check 2 is a heuristic, not a proof, and it leaves a residual hole.** "An index in
the setup" is a proxy for "an index could be involved in the error" — it over-reports
(an uppercase `UNIQUE` inside a string literal counts) and it can under-report: an
index created in the `@test` itself, or reached through a view or a CTE the scan
doesn't see, is not detected. The rule converts the common silent omission into a
forced decision; it does not make the omission impossible. Two narrower gaps in the
same area: the plan-derived check probes only the `@test`, never `@setup` (no fixture
needs it today), and the `@no-profile-lock` justification is prose that nothing
verifies.

**Both checks read the corpus, so a divergence with no fixture is invisible to them.** The
encoder class — an index whose key the B-tree key encoder refused, read differently by the
two profiles — is the worked example; its reads are fixtures (`float_key/`,
`container_key/`, tagged `@profile-lock`). A symptom need not
be an error: an **ordered scan** over an index a backend holds without the row would
answer a silent **zero rows** accelerated where the stripped build sorts a plain scan and
returns every row, so a guard written as "the symptom is an error" structurally cannot
see it — no route to such an index is known (`docs/indexes.md` § No encoder exception: every
value has a key). And a fixture for a read on which the profiles **disagree** cannot be green on
both runs: the verdict is per fixture and shared, so an `@expect-error` whose `@test`
plans to an accelerated node must carry `@profile-lock`, and the lock run then meets a
stripped build that succeeds. Such a read becomes pinnable when the disagreement closes,
not before.

A GIN index was itself profile-divergent through *index selection* — served to `ORDER BY`,
to an equality probe and to a range probe, none of which a decomposed key can answer —
until selection was taught to ask the access method; `index/gin_method_*` pins that, and
those fixtures carry **no** `@profile-lock`, because after the fix both profiles plan the
same `Filter(Scan)` / `Sort(Scan)` and leaving the tag off makes a revert red the lock
run's check 1 as well as this run's rows.
Completeness of the *tag set* is not completeness of the *coverage*
(`EDGES.md`).

**The scan covers the fixture's prose header, not only its SQL** — and that is a sharper
edge than "over-reports" suggests. `parse_fixture` routes *every* line before `@test` into
`setup`, comments included, so `setup_creates_an_index` sees the header. A fixture whose
`@test` is a bare `CREATE TABLE … UNIQUE …` therefore usually *does* trip check 2, via a
header sentence that says "UNIQUE" — with no `@setup` block anywhere in the file. Two
consequences, both counter-intuitive:

- **Never delete a `@no-profile-lock:` that "looks inert".** A `create_table_*.sql`
  fixture with **no `@setup` block at all** is routinely armed by its own prose header,
  and removing the directive turns the lock run red; others carry a directive that fires
  on nothing today — inert only until someone rewords a comment. "Has a setup block" is
  not the tell either: `@expect-error` fixtures armed *only* by prose exist both with and
  without one. Counting them here is what goes stale — a count drifts with every fixture
  added and every reworded header. **Re-derive instead**: `parse_fixture`'s routing (every line before
  `@test` that is not itself a `-- @…` directive, comments included, becomes `setup`)
  plus `setup_creates_an_index`'s three uppercase substrings.
- **Rewording a header can arm the rule.** Editing
  `create_table_dup_name_mixed_column_and_table_level.sql`'s header to mention
  `pg_constraint`'s "unique index" did exactly that. The fix is to add the directive, not
  to write around the word — the decision is worth recording either way, and prose should
  not be steered by a substring scan.

Keep the directive on this whole family regardless of whether it currently fires. Don't
manufacture an `@setup` to make it load-bearing; that changes what the fixture asserts to
satisfy a lint.

The lock covers the **stripped** direction — a fixture whose accelerated plan bypasses
predicate evaluation isn't exercising the row-filter engine in the main run at all. The
**accelerated** direction (a lossy probe coercion, a wrong index-key order) is the main
`deno task test:fixtures` run's job; the lock stays green on those by construction.
What the lock adds *over* the main run is measured, and modest — see
`docs/indexes.md` § Build profiles before treating a green lock run as evidence
about the index paths.

### `-- @param`

`-- @param` sends the `@test` query with real bound parameters over the
extended protocol — the same binary-bound path a binary-protocol driver uses — so
parameter type inference and decoding are checked against Postgres, not just
literal SQL. One directive per `$N`, indices contiguous from `$1`:

```sql
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @param $2 text[] {a,b,c}
-- @param $3 text NULL
-- @test
SELECT name FROM users WHERE id = $1 AND tags @> $2;
```

Types: `bool`, `int2`, `int4`, `int8`, `float8`, `text`, `uuid`, `jsonb`,
`timestamptz` (RFC3339), `date` (`YYYY-MM-DD`), `numeric`, `text[]`, `uuid[]`, `int4[]` (Postgres `{...}` array
text; an `int4[]` element spelled `NULL` is a NULL element)
— the canonical spellings; common aliases (`boolean`, `smallint`, `int`/`integer`,
`bigint`, `double`, `varchar`, `timestamp`, `decimal`) are accepted. `NULL`
(case-insensitive) is a typed NULL; wrap a literal in single quotes to preserve
surrounding whitespace. `@param` requires a single-statement `@test` and is
incompatible with `@text` (the simple-query protocol has no parameter binding).

**`@param` declares the value, not the bind type.** The harness binds through
tokio-postgres, which sends `Parse` with empty parameter types and takes the
server's inferred OIDs from `ParameterDescription`; the type token only chooses
which Rust value the harness constructs. A fixture therefore **cannot force a bind
type that contradicts inference** — `-- @param $1 float8` against
`WHERE numeric_col = $1` fails inside the client, because the server infers
`numeric` and the client refuses to encode an `f64` as one. The fixture never
reaches the engine, so this shape can't be used to test a coercion. To exercise a
specific bind type, make the query infer it: cast the parameter (`$1::float8`) or
compare it against a column of that type.

That is a limit on one fixture's *coercion*, and it is also a limit on a whole
**class**: a `$N` an **enum** position types. 18.6 resolves such a parameter to the
enum's own OID, reports it in `ParameterDescription`, and the client then refuses to
serialize a Rust `String` against it — `error serializing parameter 0`, measured, on
the oracle leg, so the fixture never reaches either server and no `@param` spelling
reaches the rule. There is no workaround inside the directives: casting or declaring
the parameter changes it into 18.6's `text` and the enum target is gone with it. The
class is guarded off-corpus instead, on both decode paths and through all three
instruments that reach them — `crates/pglet/tests/enum_identity_gates.rs` and the wasm
suite for `Database::execute_bound`, `pglet_server`'s `handlers.rs` Bind test for the
native server's.

**`-- @param-declared` declares the bind type.** Same grammar, one directive per `$N`,
and the two mix in one fixture: the binary leg then prepares the `@test` with
`prepare_typed`, sending each `@param-declared` parameter's type in `Parse` and `unknown`
(`705`) for each `@param` one, which both engines read as "infer this one". The declared
OID is the canonical type the keyword binds — `timestamp` declares `timestamptz`,
`varchar` declares `text`. It reaches what a type-declaring client reaches: the declared
type in parse analysis (`docs/param_binding.md` § A declared type is part of parse
analysis). Making `@param` itself declare would have moved the existing `@param` fixtures
off the inference path they test, so the declaration is its own directive.

### COPY fixtures, session time zone, expected.json

**COPY fixtures are separate.** The `copy/` fixtures use their own directive family
(`@copy-from` / `@copy-to` / `@data` / `@data-hex` / `@verify`) and are run by the `test_copy`
binary (`deno task test:copy`), not `test_fixtures` — the directives above don't
apply to them. A directive line naming none of them is a hard parse error there too, and
a directive line is what the main parser's rule above makes one, `@` followed by
`[A-Za-z0-9_]`: `-- @data-hexx` is refused, while `-- @ data-hex`, `-- @@data-hex` and
`-- @-data-hex` are not directive lines, so each is folded into the `COPY` statement as
text with the hex lines after it, and the fixture sends no data. That check reads a
`@data` block's lines as well, so COPY text of the directive-line shape is spelled with
`@data-hex`.

**Session time zone.** The harness pins both the Postgres and pglet sessions to
`TimeZone=UTC` (`connect()` runs `SET TIME ZONE 'UTC'`, which pglet accepts as the zone it
already has). pglet has one
session zone — it always stores and renders `timestamptz` in UTC — so this makes the
comparison deterministic: a `@text` `timestamptz` value renders as `… +00` on both
sides. The contract made explicit is **pglet ≡ Postgres at `TimeZone=UTC`**.

**Type mapping in expected.json:**

BIGINT values appear as strings (e.g., `"count": "3"`) because:
- JavaScript can't precisely represent integers > 2^53
- node-postgres returns BIGINT as strings for safety
- Ensures compatibility between Rust and TypeScript test suites

See `crates/pglet_tests/README.md` for full type mapping table.
