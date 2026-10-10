# Indexes and the acceleration profile

Index selection semantics — the query planner's, and the `ON CONFLICT` arbiter's —
the two supported build profiles, the guards over their *intended* byte-identical
equivalence, and the measured exceptions against it, which `EDGES.md`'s header names
(§ Build profiles carries each). The rules are summarized in the repo `CLAUDE.md`; this
file carries the full statements. GIN containment mechanics are in [arrays.md](arrays.md);
the storage-side index maintenance is [storage.md](storage.md)'s, as is what the
arbiter's *probe* then decides. A simple `UPDATE` or `DELETE` finds its target rows through
the query planner's index selection where its `WHERE` allows (§ A write's target rows); an
`UPDATE … FROM` finds them by a full scan of the table; an `INSERT … ON CONFLICT (…) DO
UPDATE` finds its conflicting row through the arbiter index.

## Index probes are exact-or-decline

With the comparison operators gated ([coercion.md](coercion.md)), an
accelerated path must not accept a probe the gate would reject, or the two
build profiles disagree. A probe value coerced to the indexed column's type may
substitute for the original **only when it is exactly equivalent**; otherwise
the planner declines the index and a row filter runs. So
`WHERE indexed_int = 1.5` returns no rows and `WHERE indexed_int > 1.5` returns
the right ones under both profiles; a lossy re-coercion would make the accelerated
build answer differently from `Filter(Scan)`. The cost is a
**performance** characteristic worth knowing: an inexact probe silently
degrades to a scan rather than erroring.

**The foreign-key probe takes the same rule, with one exception that is 18.6's.** A child
row's key is looked up in the parent's covering unique index
(`constraints::parent_tuple_exists`), and a cross-type key has to be re-encoded in the
parent's key type first. 18.6 compares the pair through the parent opfamily's cross-type
`=` where it has one — `int8 = int4`, `float4 = float8`, `date = timestamp` — and that
operator compares both sides un-narrowed, so a coercion that **moved** the child's value
would find a parent 18.6 does not match: a `float8` child `1.1` is not the `real` parent
`1.1`, and a `timestamp` child at noon is not its day's `date` parent — both `23503` on
18.6, and both accepted by a probe that narrows. `coerced_probe_tuple` declines such a
value and the parent scan decides. The exception is the pair with no such operator: the
float opfamily holds only the two float types, so an integer or `numeric` child of a float
parent is compared by **casting the child to the parent's type**, and to `real` that cast
rounds — `1.1` references the `real` `1.1`, `16777217` the `real` `16777216`. There the
rounded value *is* the comparison, on the child-side check and on the parent-side action
alike; `constraints::fk_key_matches` is the one equality both scans read, so a `DELETE`
of that `real` parent finds its `numeric` child (`23503`, or the cascade). Fixtures:
`float_key/fk_*`.

**A NULL key asks no index.** `col = NULL` is NULL for every row, so an equality probe whose
key evaluates to NULL answers no row without a lookup, as a NULL range bound does. Asking
the index was right only under the default `NULLS DISTINCT`: a `NULLS NOT DISTINCT` unique
index holds its NULL-keyed rows under a NULL key, and `SELECT id FROM t WHERE u = NULL`
over one answered that row where 18.6 and the stripped build answer none
(`index/null_key_probe_nulls_not_distinct.sql`, its `$1` twin, and
`dml_probe/delete_null_param_nulls_not_distinct.sql`).

**A range read answers no row whose key is NULL.** A comparison is not true of a NULL, so
`c < 5` holds for no NULL-keyed row, and an index answers a range by walking keys. The
default `NULLS DISTINCT` index holds no entry for such a row; a `NULLS NOT DISTINCT` one,
unique or not, holds it under a NULL key that sorts below every value's. So the walk never
starts below the least key a present value has: `index_keying::encode_range_bounds` — where
the B+tree's walk, the memory backend's and the read inside a block
(`index_range_lookup_in_tx`) all take their ends from — gives a range with no lower bound
that key as its start. Two things about that start are load-bearing. It is the least
*present* key and not the key just above a NULL one: a composite key whose leading part is
NULL sorts above a lone NULL key and below every key whose leading part is present, so the
rule holds by the key's leading tag, for a key of any arity — the start's rule alone: a
*bound* is one segment's blob, which does not sit in a composite key's order, and no
composite index is offered a range. And it is **inclusive**: the
least present key is a value's own — the empty string's, an empty `bytea`'s, an empty
array's.

A read with no bound at all — an `ORDER BY` served from the index — takes the same start,
so it answers every row of the table only where no key is NULL. The planner elides the
`Sort` where the column is *declared* `NOT NULL` (`planner::query`'s
`try_order_by_index_pushdown`); over a nullable key the `Sort` stays, and places the
NULL-keyed rows. So the elision is as sound as the declaration, and the declaration is
held where it is made: each statement that sets it over existing rows reads them (`SET
NOT NULL`, `ADD PRIMARY KEY`, `ADD COLUMN`'s back-fill), and `ALTER COLUMN … ADD
GENERATED … AS IDENTITY`, which reads none, is refused `55000` over a column not already
declared, as on 18.6. No statement this build accepts is known to leave a NULL under the
declaration; `crates/pglet/tests/identity_declaration.rs` sweeps the ones that would,
comparing each indexed column's `ORDER BY` with a sort of the table's rows. The exception
is not a statement: a store in which an earlier build accepted the identity clause over a
NULL still holds it, and its ordered read leaves that row out on the accelerated build
(`EDGES.md` row 38). What a range read leaves out is not what the index holds: uniqueness,
`ON CONFLICT`'s arbiter, the foreign-key probe and the equality probe ask for one key
(`index_lookup`), which is where a `NULLS NOT DISTINCT` index's NULL key is found — a
second NULL under a unique one is `23505`.

Measured over `(c int, v text)` under `CREATE UNIQUE INDEX … (c) NULLS NOT DISTINCT`
holding `(NULL, 'null')`, `(1, 'one')` and `(10, 'ten')`, on both build profiles and both
backends, each as on 18.6: `c < 5`, `c <= $1` bound `1` and `5 > c` answer `one`;
`c < 5 ORDER BY c DESC LIMIT 1` `one`, where a NULL-keyed row in the range would sort
first; `c < 0 ORDER BY c LIMIT 1` no row; and `INSERT … SELECT … WHERE c < 5` stores one
row. The fixtures are `index/range_null_key_*` — the range's spellings, the key types a
fixture can bind, the writes that read a range through a subquery, autocommitted and
inside a block — with the least key over a plain index beside them
(`index/range_lt_holds_empty_text.sql`, `index/order_full_table_holds_empty_*`).
`crates/pglet/tests/range_null_key.rs` holds what a fixture does not reach: the embed's
stores, a bound parameter of each key type, a block's pending rows and a reopened file
store. That the range is still read through the index is held by
`crates/pglet_sql/tests/engine_index.rs`, which counts whole-table reads, and by the plan
assertions beside the embed tests: an answer cannot show it.

## A write's target rows

**A simple `UPDATE` or `DELETE` takes the read path's equality probe only where nothing
the probe leaves unevaluated can fail, and it writes the rows it finds in `RowId` order.**
The choice is made at plan time: `planner::query`'s `dml_target_probe` asks
`plan_filter_or_index_scan` — the `SELECT` path's own selector — for the target's `WHERE`,
and keeps the node it builds as `Plan::Update` / `Plan::Delete`'s `target_probe` when that
node is an equality `IndexScan`. So a reader of the plan, the profile lock's completeness
check among them, sees that the write takes a different path on the two builds. The
executor's `scan_matching_rows` then reads the rows the key names instead of the table, and
tests each against the **whole** `WHERE`, unchanged — the probe narrows the candidates and
decides nothing. The selector offers no `IN` list as a probe, so `WHERE k IN (1, 2)` scans.

**Why the residual is a whitelist.** The scan evaluates every conjunct over every row; the
probe path evaluates the residual — what the probe did not lift — over its candidates only.
A conjunct whose evaluation can fail is the hazard: where it fails whatever the row (a
`DEFAULT`, a missing relation inside a subquery, a constant 18.6 folds while planning, such
as `1/0`) the scan raises it at the first row and the probe at none — § The starved-leaf
exception's mechanism, moved onto a write, where it reports work done; and where it fails
on some rows only (`10 / x > 0`), leaving those rows out answers where the scan raises.
So `planner::index_selection`'s `dml_residual_is_certain` admits only conjuncts whose
evaluation cannot fail once planning has accepted the statement — a bare boolean column, a
column's null test, and a column compared with a literal or a parameter — and anything else
leaves the write on the scan. Measured over a primary key `a` holding `1`, each answering
the same on both builds:

| statement | both builds | 18.6 |
| --- | --- | --- |
| `UPDATE t SET v = 'q' WHERE DEFAULT IS NULL AND a = -1` (and the `DELETE`) | `42601` | `42601` |
| `UPDATE t SET v = 'q' WHERE a = -1 AND DEFAULT IS NULL` (and the `DELETE`) | `UPDATE 0` — the `AND` stops at `a = -1` for every row (`EDGES.md` row 56) | `42601` |
| `UPDATE t SET x = x WHERE 1/0 = 1 AND a = 9` | `22012` | `22012` |
| `UPDATE t SET x = x WHERE 10/x > 0 AND a = 1`, over a second row with `x = 0` | `22012` | `UPDATE 1` — its plan takes `a = 1` as an index condition and filters `10/x > 0` over the one row it returns |

The first, third and fourth are `dml_probe/*_ahead_of_probe_*.sql`; a probe that admitted
their residual answers `UPDATE 0`, `UPDATE 0` and `UPDATE 1`.

**A parameter is the residual's one execution-time verdict**, so the executor asks it
before probing (`executor::query`'s `probe_dml_targets`): every parameter a conjunct
compares a column with — the probe's own key among them — must be bound and have a
comparison operator against the column's type. Where one does not, the write declines to
the scan, which raises the failure where it did before there was a probe: an unbound `$1`
in `UPDATE t SET v = 'v' WHERE v = $1 AND a = -1` is `22000` on both builds (18.6:
`42P02`), where a probe on `a` would have answered `UPDATE 0`. (A `$1 IS NULL` conjunct never
reaches this check: the residual whitelist above declines it at plan time.) An inexact key
declines as the read path's does.

**The order is the scan's.** The writes happen in `RowId` order — the order
`Storage::scan` returns rows in, read from the B+tree's, the memory backend's and the
transaction overlay's implementations; the trait does not state it — because their order is
observable: it is `RETURNING`'s row order, the order a `SET n = DEFAULT` draws a sequence in,
the order the per-row unique check sees the writes in, and the order a cascade is seeded in.
Every backend's equality lookup answers one key's ids in that order already (the B+tree's
entries carry the row id after the key, the memory index keeps each key's ids sorted, the
in-transaction lookup sorts), and `probe_dml_targets` sorts them anyway, so the write order
does not rest on that. Over rows `(g, k)` inserted `(1, 2)` then `(1, 1)` with `k` unique,
`UPDATE … SET k = k + 1 WHERE g = 1` succeeds, as on 18.6, where the other order would raise
`23505` (`dml_probe/update_probe_unique_shift_row_id_order.sql`; the range spellings, which
scan, are held beside it). The collection reads through a shared borrow of the store and
finishes before the first write, so a write that moves the probed key is not seen again by
its own statement.

**What declines, and why.** A range probe, for cost: a B+tree host runs every write inside
a storage transaction — an autocommit write in an implicit one — and inside a transaction a
range lookup (`index_range_lookup_in_tx`) reads the whole table, encodes each row's key and
sorts, so it saves nothing over the scan and the write would then fetch each row again.
The same lookup serves a `SELECT` inside a block: measured on a release build over a
100 000-row primary key, `SELECT v FROM t WHERE id > 99990` answers in about 15 µs in
autocommit and about 26 ms inside a block, against about 8 µs for the equality spelling in
the block. The in-transaction equality lookup reads the committed index and the
transaction's own writes only, which is why a write takes the equality probe and no other.
A GIN containment probe: its probe and guard expressions are
general scan-independent expressions evaluated once ahead of every row — none holds a
volatile call (`random()`, `gen_random_uuid()`, `clock_timestamp()`), which the selector
declines so the `OR` takes the scan and draws once a row — and its recheck is the whole
predicate, with no residual to judge apart from the containment. An `UPDATE … FROM`, which collects through its join. Every key
the selector itself declines — a partial B-tree index, an enum key column, a composite or
expression index. Two cost guards count whole-table reads: over the scan-and-filter oracle
(`crates/pglet_sql/tests/dml_target_probe.rs`) — none for a keyed write on the default build,
one on the stripped build and for an unindexed or declined `WHERE` on both — and through the
B+tree adapter inside a transaction (`crates/pglet/tests/dml_target_probe_hosts.rs`, by the
wrapper's `table_scans_in_tx`), where a write that carries a probe must read no whole table.

**An interval that names no key answers no row** — an inverted one (`k BETWEEN 4 AND 2`), or
equal ends not both included (`k > 3 AND k < 3`) — on every backend. The B+tree's half-open
walk finds nothing in it; the memory backend's lookup returns nothing before asking its map,
whose `range` panics on such an interval (`index/range_probe_names_no_key*.sql`,
`dml_probe/delete_inverted_range.sql`, and both backends in
`crates/pglet/tests/dml_target_probe_hosts.rs`).

## Index selection must ask what an index *is*, not only what it covers

A GIN index stores one **decomposed** entry per element / object key, not one
per row, so it answers containment and nothing else.
A selector that filters on table ∧ single bare column ∧ non-partial and never on
**method** hands a GIN index to all three of `single_column_index_for`'s consumers:
`SELECT id FROM t ORDER BY j` over `CREATE INDEX … USING gin (j)` then returns each
row once per key, in key order (`1,3,3,3,2,1` for three rows against Postgres's
`2,1,3`, and `LIMIT 1` a single entirely ordinary-looking wrong row), while a bound
`WHERE j = $1` / `j > $1` selects the same index and probes its decomposed entries
with a whole-value key, where Postgres answers rows. **Every statement in that reproduction is accepted by both
engines** — no refused DDL, no failed back-fill, no empty index — and that is what
makes it pinnable by ordinary differential fixtures (`index/gin_method_*`). The
gate asks
`executor::index_method_for`, the *same*
derivation that builds the storage `IndexSpec`, so selection cannot disagree
with what storage maintains, and `gin_index_for` asks that same function for the
complementary answer rather than re-deriving it. The complement is of the
**method gate**, not of the selector: `single_column_index_for` narrows further
on `predicate.is_none()`, while `gin_index_for` does not — it offers a partial
GIN index and leaves the predicate to `find_index_containment`'s implication
check, which is why a *partial B-tree* index is offered by neither selector and
a partial GIN by one. Two things the gate deliberately
does **not** do, both because the question is the method and not the type. A
*unique* index over a container column is a **B-tree** (GIN cannot enforce
uniqueness), so it stays offered, and serves an equality probe, a range and an
ordered scan through its key (§ A container key). And the type-shaped near-miss
("skip container key columns") would take those reads off the index without
reddening an `index/gin_method_*` fixture, which is why the one unit test asserting
the unique-container index is *still* offered is the discriminator, not a
redundancy. The ordered-scan pushdown is reached only by a GIN-indexed container column
read outside `@>` and declared `NOT NULL` outside a partial predicate — the shape the
`index/gin_method_order_by_*` fixtures carry.

### ...and what the key column's *order* is

The same question one step further in: an index answers an ordering **by the key
encoding's order**, and the B-tree key encoder has no enum arm — it encodes a label's
bytes, as it does any `Value::Text`. Since an enum orders by its members' *positions*
([enums.md](enums.md)), an index over an enum column is ordered by something the engine
does not order by. Served from such an index, the reads that turn on that order are
answered *differently*, not faster: measured over `mood AS ENUM ('sad','ok','happy')`, whose declared order is the
reverse of its lexical one, `ORDER BY m` answered `happy, ok, sad` from the index against
the unindexed table's `sad, ok, happy`, and `WHERE m > 'sad'` answered all three rows
against the correct two — an **index-presence** divergence on one binary, and a
build-profile divergence besides, the stripped planner declining every index and
row-filtering. An *equality* read is the other kind: the encoding orders it wrong and
still groups it right, so the index serves it the same answer, faster.

`single_column_index_for` declines an enum key column, which drops it for all three
consumers at once. Two consequences worth carrying. The cost is **performance** only: the
row filter is exact, and label-byte *equality* is still enum equality (18.6's `enum_eq`
is bare OID identity), so nothing an index could have answered is lost. And the decline is
what makes an enum column's **partial** index harmless to read — the B-tree selector
already excludes a partial index, so the one place its membership still matters is a
*unique* one, whose enforcement takes the member order from `IndexSpec::column_enum_orders`
instead ([enums.md](enums.md) § The two write paths).

**"Performance only" is not "free", and the two consumers differ by two orders of
magnitude.** Measured on a **release** server, 20 000 rows over a five-member enum and a
`text` twin holding the same labels, both indexed, medians of five:

| read | `text` (index) | enum (declined) |
| --- | --- | --- |
| `ORDER BY … LIMIT 10` | 0.049 ms | 9.27 ms |
| `count(*) WHERE … = 'calm'` (4 000 rows match) | 2.6 ms | 5.2 ms |

The ordering read is ~190× and the gap is **asymptotic** — a whole-table `O(n log n)`
sort where the index walks `O(k)` and stops — so it widens with the table. The equality
read is ~2×, a constant factor over the same result set, and that is the one the decline
was reasoned about ("nothing an index could have answered is lost", which is a statement
about *answers*). A cheaper `enum_rank` would not move either figure much: it is
`order.iter().position(…)`, O(|members|) per side per compare, and a member list is
short — but it is on the sort's inner loop, so a type with hundreds of members would
make the ordering row worse than this table shows. Unmeasured at that size.

The alternative — encoding the member position in the key — is a persisted-shape change
and forecloses `ALTER TYPE … ADD VALUE … BEFORE/AFTER`, which is why 18.6's own
`pg_enum.enumsortorder` is a `float4`.

**What the decline must not become is a load-bearing premise.** The un-accelerated
fallback beside the index reads (`executor::query::scan`'s `scan_filtered_by_key`, taken
when a probe is inexact) filters and sorts by the key column's member order, derived
**inside** the function by `enum_key_order` rather than passed in — a parameter both callers
pass as the `None` it always is would be a wiring either could quietly drop, with nothing
reddening — so a later narrowing of the decline (allow equality, decline range and order)
cannot reopen a mis-ordered read on the accelerated profile alone; label-byte filtering
and sorting (`eval_binary_op`, `values_compare`) would be correct *only* while the decline
keeps every enum away. No plan reaches that arm while the decline stands, so
`enum_key_fallback_tests` executes it directly, and nothing else does: forcing the
derivation to `None` reddens those tests and nothing in the fixture corpus or the
`cargo test` workspace.

## A float key

`float4` and `float8` key through one arm of `Value::encode_index_key`
(`pglet_core`'s `float_order_key`): the value widened to `float8`, as eight big-endian
bytes in the order 18.6's `float8_cmp_internal` gives — `-Infinity`, the negatives, zero,
the positives, `Infinity`, `NaN`. A non-negative value's sign bit is set and a negative
value's bits are all inverted, which makes the IEEE-754 pattern unsigned-comparable; the
two canonicalizations in front of that are what make byte equality the opclass's equality:

- **`-0` takes `0`'s key.** `float8eq` calls them equal, so a unique key holding one
  refuses the other and a probe for either finds both.
- **Every NaN takes one key, above `Infinity`'s** — either sign, any payload. This is
  load-bearing and not hygiene: `'Infinity'::float8 - 'Infinity'::float8` carries the sign
  bit on 18.6's own platform (`float8send` answers `\xfff8000000000000` against `'NaN'`'s
  `\x7ff8000000000000`), the two compare equal there, and an arithmetic NaN's sign is not
  fixed under wasm.

Keying a `float4` as the `float8` it widens to — exactly, and monotonically — puts the two
widths in **one key space**, which is where the engine's own comparator already puts them:
`cmp_f64_total` (`executor/expr/compare.rs`) is the float comparator behind `values_compare` and
`values_equal`, and it is `float8_cmp_internal`. So the key order *is* the comparator's
order, an ordered scan over a float key returns what the `Sort` would have, and the
exact-or-decline rule above needs no float arm of its own: a probe value coerced to a
`real` key's type substitutes only when it compares equal to the written value **as
`float8`**, so `real = 1.1` (a `numeric`, compared as `float8` on 18.6) declines the index
and row-filters to no row, while `real = 1.5` probes. A `float8` key probed by a `numeric`
or an `int8` takes the value's `float8`, which is what 18.6 compares, casting the other
side to `float8`.

Nothing decodes a key — an entry stores no value — so there is no decoder, and the
encoding added no tag: a float takes the same present-segment framing every scalar does.
It is not a store-format break either. A store written by a build that refused a float
key holds no present float key segment, because every door that writes an entry — a
write's maintenance, a build's back-fill, a rebuild, a GIN decomposition — asks this one
encoder; so the arm gives meaning to bytes no such store contains, and such a store's
float index, holding at most NULL entries, is simply writable
([storage.md](storage.md) § Store versioning).

A GIN index over `float8[]` / `real[]` keys each element the same way. Its candidate
test for a block's own pending rows (`gin::gin_contains_keys`) compares `Value`s, whose
`==` is the key equality for every element but a NaN — unequal to itself there, one key in
the index — so a NaN key is matched by any NaN element.

Fixtures: `tests/fixtures/float_key/`, each a measured statement — the routes (plain,
composite with the float in either position and in neither position's table order,
expression, partial, unique, `PRIMARY KEY`, `ADD COLUMN`, `ADD CONSTRAINT`,
`ON CONFLICT`, GIN, foreign key), the equality classes, and the probes a `real` key must
decline.

## A container key

A `jsonb` and an array value each key through an arm of `Value::encode_index_key`, and
the rule both are written to is one sentence: **the key's byte order is the comparator's
order, and its byte equality the comparator's equality** — `executor::expr`'s
`values_compare`, which is what a `Sort`, a `Filter` and the stripped build answer by. An
index answers by the bytes, so that is the condition under which the two build profiles
agree over a container key, and under which a unique index's duplicates are `=`'s.

- **An array** is its elements in order. Each element is a presence tag and the
  element's own key — `0x01 ++ key`, or a bare `0x02` for a NULL element — and nothing
  ends the array: the framing around a key does, so a shorter array is a prefix of one
  that extends it and sorts below it. That is 18.6's `array_cmp`: elements before length
  (`{1,2,3}` sorts below `{2}`), a NULL element above every value and equal to another
  NULL (`{1,NULL}` is a duplicate of `{1,NULL}`, where a NULL **key** repeats freely). A
  `text` or `bytea` element's key is escaped and terminated (`0x00` → `0x00 0xFF`, then
  `0x00 0x00`), since one string can be a prefix of another; every other element key is
  fixed-width or ends itself, and is written bare. The element
  key is the element type's own, so a float element's `-0` is `0` and every `NaN` one key
  (§ A float key), and a `numeric` element's scale folds.
- **A `jsonb`** is 18.6's `compareJsonbContainers` order: a type tag in rank order — null,
  string, number, boolean, array, object — then the content. An array is its element count
  ahead of its elements (**length first**, the opposite of a SQL array), an object its pair
  count ahead of its pairs in stored order, each key's bytes escaped ahead of its value. A
  number is its `numeric` order key, which folds scale, so `1` and `1.0` are one key at any
  depth. One value sits outside the rank: 18.6 stores a top-level scalar as a one-element
  array, so the **empty top-level array** sorts below every top-level scalar, and it takes
  a tag below the others.

**What holds the rule.** `crates/pglet_sql/tests/index_key_order.rs` compares
`memcmp(key a, key b)` with `values_compare(a, b)` over every pair of a generated
population, `Equal` included — nested `jsonb`, and arrays of each element type with a
key, NULL elements among them. That checks pglet against itself; 18.6's order is held by
`pglet_core`'s measured ladders (`types::index_key`'s tests) and by the fixtures,
`tests/fixtures/container_key/`, which run on both build profiles. The comparator reads
two integers as integers, never through `f64`, through which two `bigint`s past 2^53 that
differ by one would compare equal — in an `int8[]`, and in a plain `ORDER BY`, `min`,
`count(DISTINCT …)` and `>` filter alike (`types/bigint_compare_past_float_precision.sql`).

**String order is the comparator's, which is bytes.** A `jsonb` string, an object key and
a `text[]` element order by their UTF-8 bytes on both build profiles, where 18.6 orders
them by the database's collation (`EDGES.md` row 43). Equality is unaffected — a
deterministic collation breaks ties by the bytes — so a unique key and an equality probe
agree with 18.6; an `ORDER BY` or a range over strings a collation orders otherwise does
not, with or without an index.

**The routes.** Each index over a container key that `executor::index_method_for`
derives a B-tree for is built and maintained as one: a declared `UNIQUE` / `PRIMARY KEY`
at each spelling, `CREATE UNIQUE INDEX` over a bare column, with or without a predicate,
and a composite or expression key, unique or not. A non-partial unique or primary key
over a bare container column is offered to an equality probe and a range, and where the
column is `NOT NULL` to an ordered scan. A written container value is a `Cast` or an
`Expr::Array`, which index selection does not lift, so the probe that reaches the index
is a bound `$N` — the extended protocol's — and a literal's row-filters on both
profiles. A GIN index over `jsonb[]` keys each element by the `jsonb` arm, so a
containment probe matches by `jsonb`'s equality; its dedupe of a row's keys compares
`Value`s, whose `==` folds a `jsonb` number's scale as the key does.

**The one shape refused** is a non-unique B-tree over a single bare container column,
`CREATE INDEX ON t (c)`: `index_method_for` derives GIN for that shape, the catalog
records no method to say otherwise, and `executor::ddl`'s `validate_btree_key_type`
refuses the declared B-tree rather than build a GIN index under its name (`EDGES.md`
row 25).

**An enum array** has no column form. As an expression key (`(ARRAY[m])`) it keys each
element's label bytes — right for equality, and not the enum's order, which is member
position. Nothing reads order through it: an expression index serves no ordered scan and
no range (`container_key/enum_array_expression_key_*`).

**No store-format break.** A store written by a build that refused these keys holds no
container key segment — every door that writes an entry asks this one encoder, and a
refused write or build commits nothing — so the arms give meaning to bytes no such store
contains ([storage.md](storage.md) § Store versioning).

## An `ON CONFLICT` inference list is a set, and matching an index is set equality

A second selector reads the catalog's indexes, and it answers a different question from
the query planner's: which unique index does `ON CONFLICT (…)` arbitrate against?
`match_conflict_index` (`executor/constraints.rs`) filters the relation's enforced unique
indexes and takes the first whose key parts and partial predicate match the clause's
target — as sets, since an **element-wise** comparison of the two key lists would make
the match a claim about order and about repeats that 18.6 does not make.

18.6's is `infer_arbiter_indexes` (`optimizer/util/plancat.c`), whose header states the
rule outright: an exact match on columns/expressions is required, *although they can
appear in any order*. It splits both sides the way the catalog stores an index — plain
attributes into a `Bitmapset`, key expressions into a list — and compares each half as a
set: `bms_equal` on the attributes, `list_member` one way and `list_difference` the other
on the expressions, under a comment saying that arrangement does the right thing when a
unique index redundantly repeats a key **or** an inference clause does. Order is
irrelevant and a repeat collapses, on each half.

pglet reproduces the split where the comparison is rather than where the index is stored,
because its catalog keeps a mixed key as one expression list (`IndexKey::Exprs`) rather
than as an attribute vector plus an expression list. `split_key_parts` partitions a key
list into the bare-column names and everything else, and `same_set` is the two-way
containment both halves take. Each of these is a match on both engines, and `42P10`
under an element-wise compare:

- `ON CONFLICT (a, a)` and `("a", a)` against a `PRIMARY KEY (a)` — a repeat collapsing,
  and the quoted spelling collapsing with it (both sides went through the parser's fold,
  so there is no casing left to reconcile; a quoted name that folds to something else is
  a *different* column and still refuses).
- `(b, a)` against a `UNIQUE (a, b)` — **reordered, with no repeat at all.** This is the
  member that makes it a set rule rather than a duplicate rule: a fix scoped to collapsing
  duplicates leaves it refusing. `(a, a, b)` against the same index is the repeat half
  over a composite key.
- `(y, lower(x))` and `(lower(x), lower(x), y)` against a
  `UNIQUE INDEX ((lower(x)), y)` — the expression half takes the same set rule, so a
  rule over the column half alone is half of one.
- `(a)` against a `UNIQUE INDEX (a, a)`, which reads the rule from the **index** side:
  a two-segment index over one column is accepted by both engines (a constraint's key
  list refuses a repeated column, an index's does not — [ddl.md](ddl.md)), and its
  attribute set is then `{a}`. The `(a, a)` spelling of that target matches too — under
  an element-wise compare only by the accident of the two lists being equal as sequences.
  Enforcement over such an index agrees: a second row is `23505` on both.

**Set equality is still equality**, and the two directions of the containment are
separately load-bearing: `(a, a)` against a `UNIQUE (a, b)` is caught by the index's `b`
not appearing in the target, `(a, b)` against a `PRIMARY KEY (a)` by the target's `b` not
appearing in the index, and both are `42P10` on both engines. A one-way test loses one of
them. An element-wise compare refuses both too, for other reasons — the first by the
element mismatch at the second position (the lists being the same length), the second by
the length check — so what says the refusal stands for the *right* reason is the accept
twin each has: `(a, a, b)` and `(a, a)` against a `PRIMARY KEY (a)` respectively.

The predicate half is compared by structural equality on the whole clause, so a partial index is matched by a reordered target carrying its predicate
and a bare target still fails to match one. That 18.6 requires only *implication* there
is a separate divergence, recorded in [compatibility.md](compatibility.md)'s `ON CONFLICT` row.

A target part that is not a bare unqualified column falls on the expression side, where
it matches only an index holding that very expression and nothing a plain-column index
offers — which is where a **qualified** reference lands. `ON CONFLICT (u.a)` is `42P10` here and a grammar `42601`
on 18.6, whose `index_elem` takes a bare `ColId`, a function call or a parenthesized
expression and no qualified name; the split does not reach that divergence, nor the
arbiter's columns, which stay unresolved and ungated (an inference list naming a
column the relation lacks is `42P10` here against 18.6's `42703`).

## An index entry is sized as 18.6 sizes it

Every entry a row writes into an index — at every door that writes one — is sized as
18.6's `index_form_tuple` lays it out, and refused `54000` where 18.6 refuses it, with
18.6's message, short of the four residuals below. `executor::index_row` holds the one
size: an 8-byte header, a null bitmap when any key part is NULL (so a NULL part can push a
row over a limit its absence would not), each non-NULL part at its `pg_type` alignment —
an **enum** part four bytes where pglet knows the part is an enum, which it does not behind
a written cast — and a varlena of at most 126 bytes packed to a one-byte header. The
refusals are 18.6's three: the tuple header's 13-bit size field (`index row requires N
bytes, maximum size is 8191`), a B-tree's item limit (`index row size N exceeds btree
version 4 maximum 2704 for index "i"`, sent with 18.6's hint and `s`/`t`/`n`,
[errors.md](errors.md)), and a GIN entry's (`… exceeds maximum 2712 …`), each GIN entry a
one-part tuple of one decomposed array element. A `jsonb` GIN key is not asked — 18.6's
`jsonb_ops` hashes any long key or value. A `jsonb` part and an array part are sized as
18.6 stores them (`index_row`'s `jsonb_data_len` and `array_data_len`): a `jsonb` as its
container — a four-byte header, a four-byte entry per element, a number as its `numeric`
under a four-byte header on a four-byte boundary — and an array as its header, one
dimension, a null bitmap when an element is NULL, and each element at its type's
alignment, a varlena element under a four-byte header whatever its length.

**Which refusal wins is 18.6's order.** Within one index, the size field is asked, then an
enforced unique index's duplicate probe, then the item limit (`btinsert` forms the tuple,
`_bt_doinsert` probes before `_bt_findinsertloc` sizes). Across a table's indexes, one
index at a time in creation (OID) order — so a plain index created first refuses its
oversized key ahead of a later unique index's duplicate, and the other way round the
duplicate wins. Against the row's other refusals, after NOT NULL and CHECK and before the
row's own foreign key. `constraints::enforce_index_entries` is that loop, over
`constraints::prepare_indexes`' every-index set, at `INSERT`, `UPDATE`, `ON CONFLICT DO
UPDATE` and `COPY`. A target-less `ON CONFLICT DO NOTHING` arbitrates before any entry is
formed, as 18.6's does, so a readable conflict skips a row whatever its size.

A **foreign-key action** fires once the statement's own rows are written — 18.6 runs it as
an after-row trigger — so a written row's own NOT NULL, CHECK and index refusals outrank
every refusal an action raises. The action then writes its child rows one at a time, each
checked and entered into the child's indexes before the next: NOT NULL, CHECK, then each
index entry, for a `CASCADE`'s repointed row and a `SET NULL`'s nulled row alike — a
repointed key can be NULL, and can collide under a unique index that is not the referenced
key's. An `UPDATE`'s and an `ON CONFLICT DO UPDATE`'s actions do the same, writing each
child row through `constraints::enforce_row_checks` as they go (`executor::dml`'s
`FkTriggerQueue`), so a row
two actions reach is rewritten from the first one's write and every probe reads storage as
the actions have left it. A `DELETE` plans its actions before it removes a row, so it asks
each child row those questions in the plan, through
`constraints::enforce_cascaded_index_entries`: a row two actions reach is rewritten from the
first one's image, and the duplicate probe reads a row an action has already rewritten by
that image rather than by the entry storage still holds for it — an index created first
finds a planned sibling's duplicate before a later one sizes the row, while a row's own
earlier image, and the entry of a row the statement removes, are no duplicate.

A **build** — `CREATE INDEX`, `ADD CONSTRAINT UNIQUE`/`PRIMARY KEY`, `ADD COLUMN … UNIQUE`
— refuses as 18.6's build does (`constraints::scan_index_build_sizes`): the size field as
the rows are read, the duplicate from the sort, the item limit as the sorted entries load,
so of two oversized entries it names the one its key sorts first — an enum part by its
members' positions. An `ADD PRIMARY KEY` verifies its columns NOT NULL after the whole
build, by the later table scan 18.6 runs, naming the first NULL column in the table's
order. An `ADD COLUMN` builds before the column's CHECK when 18.6 does not rewrite the
table; when the default is per-row volatile 18.6 rewrites it, checking each row NOT NULL
then CHECK as it writes it, and builds after — both orders measured, and the column's
foreign key checked after the build either way.

**The storage maintenance path asks the same sizes of every entry it adds**
(`IndexKeying::encode_row_entries`, through the same `executor::index_row` functions), on
both backends and inside a transaction at the statement, index by index in creation order.
It is a belt behind every door the executor gates. (An index **rebuild** re-derives entries
already sized, and does not ask again.) It adds pglet's one limit of its own, a key too long
for an internal B+tree page to carry as a separator (4 044 bytes), which 18.6's layout does
not share for a NUL-dense `bytea`, a long `numeric`, an array of small integers and a long
`jsonb` GIN token (`EDGES.md` row 192), and which a build raises after every refusal 18.6
would raise there — a `jsonb` GIN token excepted, which the executor's build scan does
not read and the storage build itself refuses; and it compresses nothing, so a key 18.6
holds only compressed — a `text`, a `jsonb` or an array — is refused at its full length
(row 191). Both are loud. The two silent residuals are members of rows of their own: a
`char(n)` key, sized as the `varchar` pglet stores rather than padded as 18.6 sizes it
(row 95), and an enum key part behind a written cast, sized by its label (row 104). A build
naming one of several oversized entries names the first ascending, text by its bytes, so a `DESC`
key part, which the catalog does not keep, and text 18.6 sorts in the oracle's
`en_US.UTF-8` collation name another row's size (rows 92, 43). A build reads the table,
and an `ADD PRIMARY KEY`'s NOT NULL scan reads it, in row-id order, which an `UPDATE`
leaves in place, where 18.6 reads its heap, in which the updated version moves to the end;
so once an `UPDATE` has moved a row, the size field can name another row's size, the
build's `23505` another duplicate, and the `23502` another column (row 194). And a
multi-row `INSERT` whose earlier row breaks a foreign key is that key's `23503` here,
where 18.6, checking foreign keys once the statement has run, answers a later row's
refusal (row 166).

The fixtures are `tests/fixtures/index_size/` and, for `COPY`, `tests/fixtures/copy/`:
most boundaries held as an accepted and a refused fixture a character apart, and the orders
measured at `INSERT`, `UPDATE`, `COPY`, a foreign-key action and each build. The refusal's fields, which the harness scores as
agreement when pglet omits them, are pinned by `crates/pglet/tests/index_row_size.rs` and
`pglet_core`'s `error_fields` tests.

## Build profiles

Two supported build shapes over one core, selected by the default-on
`index_acceleration` Cargo feature (declared on `pglet_sql`, forwarded through
`pglet` / `pglet_server` / `pglet_bench` / `pglet_tests` / `pglet_wasm`; the
workspace pins `pglet_sql` `default-features = false` so a consumer can drop
it):

- **default** — query index *acceleration* ON: the planner's index selection
  and the `IndexScan` / `IndexRangeScan` / `GinScan` read paths.
- **stripped (`--no-default-features`)** — acceleration OFF. The planner emits
  `Filter(Scan)` for every predicate, so an index is *intended* to change only
  *speed*, never a *result* — the differential harness stays byte-identical
  (index *objects* are still built, maintained, and introspectable, and
  PK/UNIQUE/FK enforcement — the unique-index probe — is independent).

**Measured exceptions stand against that intent.** Index selection decides the plan, and
a verdict pglet reaches by walking the plan, or where it evaluates a row or builds a
probe, can follow it. § The starved-leaf exception below holds measured members where one
build answers or accepts and the other refuses, as `EDGES.md` row 276 does for a
parameter's null test timed by the plan; and § The construct-name exception holds one that
moves the construct word inside an error message. In a store where an earlier build
accepted `EDGES.md` row 38's identity clause over a NULL, both builds answer and an
ordered read on the accelerated one leaves that row out. The members named are the
measured ones, not a bound.

### No encoder exception: every value has a key

Every value has a B-tree key (§ A float key, § A container key), so no index probe raises
an encoder refusal on the accelerated build where the stripped build row-filters: a bound
`WHERE j = $1` over `j JSONB UNIQUE`, and an `@>` over a GIN index of any element type,
answer as 18.6 does on both profiles (`tests/fixtures/float_key/`,
`tests/fixtures/container_key/`). An encoder refusal reachable from a probe would be a
divergence no fixture can hold: an `@expect-error` fixture's `@test` that plans to an
accelerated node needs `@profile-lock`, and the lock run then meets a stripped build that
succeeds.

A GIN containment probe whose array is of **another element type** than the indexed
column's (`numeric[] @> ARRAY[1]`) is not looked up in the index as though its keys were
the column's — that answers `(0 rows)` on the accelerated build, against the stripped
build's and 18.6's `42883`. `execute_gin_scan` declines a probe whose type is not the key
column's — exact-or-decline — and hands the recheck `Filter` every row, so the `Filter`
answers for the pair on both profiles (`container_key/gin_array_other_element_type_probe_loud.sql`).

An *ordered scan* over an index **missing entries for rows it covers** would answer
fewer rows than the stripped build's sort. No route to such an index is known — a failed
build registers nothing on either backend (`docs/ddl.md` § A failed index build), and a
refused single-row write leaves no row on either backend. Read that as measured over the
routes tried — the failed back-fill, the autocommit write, the write inside a block and
the build inside a block — and not as a proof that no route remains. The property is
*missing*, not *empty*: an index that holds no entries because it covers no rows is
correct.

### The starved-leaf exception

**This exception involves no refused type**: an ordinary `INT` key under a
plain B-tree index, over a statement carrying a leaf pglet refuses only when it
is *evaluated* — a query-position bare `DEFAULT` or an unbound `$N`
(`EDGES.md` row 56). Neither gate runs at plan time: the `DEFAULT` refusal is
`executor/expr/eval.rs`'s `Expr::Default` arm and the `$N` one is `EvalEnv::param`'s
out-of-range error, so both are reached only where evaluation goes — and *what a
node scans* is a plan property that index selection decides. Where the
accelerated build's probe finds no row, the leaf beside it is never evaluated
and the statement answers; the stripped build's `Filter(Scan)` reaches a row and
raises. So `EDGES.md` row 56's own loud control — `WHERE DEFAULT IS NULL AND
a = -1`, loud because the scan reaches a row — holds only while `a` is
**unindexed**.

Measured on **distinct** release binaries (`--no-default-features` and default,
copied apart and run on their own ports — the two builds write the same path, so
a server started after the other build is the wrong profile), over
`ti(a INT, v TEXT)` with `CREATE INDEX ti_a ON ti (a)` holding `(1, 'x')` (a
`PRIMARY KEY` or a `UNIQUE` on `a` reproduces every row of the table),
`te(a INT NOT NULL, v TEXT)` likewise indexed, and `t(a INT, v TEXT)` unindexed:

| statement | accelerated | stripped | 18.6 |
| --- | --- | --- | --- |
| `SELECT v FROM t WHERE DEFAULT IS NULL AND a = -1` — `a` unindexed | `42601` | `42601` | `42601` |
| the same over `ti` — and the `a = -1 AND DEFAULT IS NULL`, `a > 100 AND …` and `a BETWEEN 50 AND 60 AND …` spellings | **`(0 rows)`, `v` → `25`** | **`42601`** | `42601` |
| `SELECT v FROM ti WHERE $1 IS NULL AND a = -1` (simple leg) | **`(0 rows)`, `25`** | **`22000`** | `42P02` |
| `SELECT DEFAULT AS d FROM te ORDER BY a LIMIT 0` — no predicate at all; also the `DESC` and `SELECT $1 AS p` spellings | **`(0 rows)`, `25`** | **`42601` / `22000`** | `42601` |
| `SELECT v FROM t WHERE EXISTS (SELECT 1 FROM ti WHERE ti.a = -1 AND DEFAULT IS NULL)` | **`(0 rows)`, `25`** | **`42601`** | `42601` |
| `SELECT v FROM ti WHERE DEFAULT IS NULL AND a = 1` — the probe **matches** | `42601` | `42601` | `42601` |

Three routes reach it, and the third needs no predicate: an **equality/range
probe** (`=`, `>`, `BETWEEN`), a **GIN containment probe**
(`SELECT a FROM tg WHERE tags @> ARRAY['zz'] AND DEFAULT IS NULL` over
`tg(a INT, tags TEXT[])` holding `(1, ARRAY['p'])` under a `USING gin (tags)`
index — silent accelerated, `42601` stripped, while the matching
`@> ARRAY['p']` is `42601` on both), and the **ordered-scan
early-stop** (`Sort` elision + scan early-stop over a NOT-NULL indexed key under
a `LIMIT`), where the leaf is in the *projection* and no predicate is involved
at all.

It takes an index the planner actually **selects**, not merely an index: a
partial index the query does not imply (`CREATE INDEX tp_a ON tp (a) WHERE a >
100`, queried at `a = -1` — and, measured, at `a = 200` too), an index on a
*different* column (`CREATE INDEX tv_v ON tv (v)` under
`WHERE DEFAULT IS NULL AND a = -1`), an `ORDER BY` over an unindexed key or with
no `LIMIT`, and a bare `LIMIT 0` with no `ORDER BY` all leave the scan whole and
the statement `42601` on **both** profiles. The disjunctive
`WHERE a = -1 OR DEFAULT IS NULL` is likewise `42601` on both. And **a write takes
no probe over such a leaf**: `UPDATE ti SET v = 'q' WHERE DEFAULT IS NULL AND a = -1`
and the `DELETE` spelling are `42601` on both profiles, over the same index that makes
the `SELECT` silent, because the residual a write's probe leaves must be unable to fail
(§ A write's target rows).

This is a measured build-profile counterexample over an ordinary `INT` key and a
plain B-tree index, no refused type anywhere. It is the
**same mechanism** `planner/coercion.rs`'s `check_expr_semantics` doc records
for operand legality — *"the accelerated build lifts such a predicate into an
index probe, whose gate compares the declared types and does fire on an empty
table — so the same query answered differently on the two build profiles"* —
which that gate avoids by deciding the verdict at plan time. These two leaves have
no plan-time gate; the remedy is that plan-time binding arm, which reorders
statement-level error precedence.

**The `$N` leaf also diverges the other way, where it is the probe value itself.** The
accelerated build raises for the probe's `$N` over a table with no row, where the stripped
build's `Filter(Scan)` has no row to evaluate the comparison over. Measured on distinct release binaries over an **empty** `pe(k INT PRIMARY KEY, v TEXT)`,
simple leg, no parameters:

| statement | accelerated | stripped | 18.6 |
| --- | --- | --- | --- |
| `SELECT v FROM pe WHERE k = $1` | **`22000`** | **`(0 rows)`, `25`** | `42P02` |
| `SELECT v FROM pe WHERE k = $1::int` | **`22000`** | **`(0 rows)`, `25`** | `42P02` |
| `SELECT v FROM pe WHERE v = $1::text` — `v` unindexed | `(0 rows)`, `25` | `(0 rows)`, `25` | `42P02` |
| either `k` spelling once `pe` holds `(1, 'a')` | `22000` | `22000` | `42P02` |

The cast spelling is a probe because `is_row_independent` admits a `Cast` directly over a
`$N` — the shape a parameter whose type the client declared takes (`docs/param_binding.md`)
— read from the code; a build whose `is_row_independent` declined the cast measured
`(0 rows)` accelerated for that spelling. Here the stripped build is the silent one and the accelerated
build the loud-but-miscoded one, the opposite of the first table above, so no single profile is
the conservative one for this leaf. It is the same missing plan-time gate, and closes with
it.

**Other eval-time verdicts starve the same way.** The leaf need not be a `DEFAULT` or a
`$N`; these were measured with a refusal and with an operator verdict in its place, on
the two debug builds (default and `--no-default-features`, copied apart) and through the npm package,
which answers as the accelerated build does:

| statement | accelerated | stripped | 18.6 |
| --- | --- | --- | --- |
| `SELECT id FROM t WHERE a = ANY ((SELECT x FROM e)) AND id = 5` — over `t (id INT PRIMARY KEY, a INT)` holding `(1, 1)`; the `id > 5` spelling likewise | **`(0 rows)`** | **`0A000`** | `(0 rows)` |
| the same with `id = 1` — the probe **matches** | `0A000` | `0A000` | the row |
| the same conjuncts written `id = 5 AND a = ANY (…)` | `(0 rows)` | `(0 rows)` | `(0 rows)` |
| `SELECT 1 FROM u WHERE id = $1 AND t = $1` — over `u (id UUID PRIMARY KEY, t TEXT)`, **empty** or holding one row of another `id`, `$1` bound a uuid's text | **`42883`** | **`(0 rows)`** | `42883` |
| the same over such a `u` with no index | `(0 rows)` | `(0 rows)` | `42883` |

The first is the sub-select form of `ANY`, which is refused where it is evaluated
(`EDGES.md` row 180's bullet); the stripped build's `Filter` evaluates the conjuncts in
written order over the one row, and the accelerated build's probe finds none. The second
is a parameter two positions type differently (`EDGES.md` row 148), where the accelerated
build is the one that agrees with 18.6. A `DELETE` and an `UPDATE` over the first `WHERE`
are `0A000` on both builds: a write takes no probe for a `WHERE` that holds a subquery
(§ A write's target rows).

No fixture puts either leaf over an index, and `@profile-lock` could not hold
one if it did: a fixture can pin one profile's answer or the other's, never
both. The full member set — every query and write position probed, their
controls, and the members that starve for reasons other than a shortened scan —
is `EDGES.md` row 56.

### The construct-name exception

**This measured case moves no result at all.** A boolean gate names the
construct it refused (`argument of <CTX> must be type boolean`), and at a
**nested** query level which construct it names follows the plan. The clause-root
gate at that level's own lowering *defers* — a correlated reference types `None` —
so the walk that judges it is `planner/correlation.rs`'s `walk_level`,
which gates a `Plan::Filter`'s predicate as `BoolContext::Where`. By then
`plan_filter_or_index_scan` has lifted one conjunct into an
`IndexScan`/`IndexRangeScan` and left the rest in the `Filter`, so where the
mis-typed conjunct is what is left, the node gated as a clause root was an `AND`
argument as written.

Measured on distinct release binaries (default and `--no-default-features`,
copied apart and run on their own ports) over `ni1(a INT)` and `ni2(k INT)`, with
`SELECT 1 FROM ni1 WHERE EXISTS (SELECT 1 FROM ni2 WHERE ni2.k = 1 AND ni1.a)`:

| schema | accelerated | stripped | 18.6 |
| --- | --- | --- | --- |
| no index on `ni2.k` | `AND` | `AND` | `AND` |
| after `CREATE INDEX ni2_ix ON ni2 (k)`, nothing else changed | **`WHERE`** | `AND` | `AND` |

Every answer in that table is `42804` and the statement is refused
either way, so the byte-identical *result* intent is untouched here; what the
profiles disagree about is the sentence. The range-probe spelling and the reversed
conjunct order diverge the same way, while the statement's own top level, an
uncorrelated nested level, an `OR` in place of the `AND`, a GIN containment probe
(which rechecks the whole original predicate) and a `NOT` between the conjunct and
the leaf all agree — the last of those placing the lost context at the **root** of
the residual and nowhere else. `EDGES.md` row 126 carries the members, why no
fixture can hold it, and the guard that does.

### The size lever, and the invocations

Only the acceleration *consumption* drops (and, via dead-code elimination, the
now-unreachable GIN/range storage probes), trimming ~2.4% off the wasm
(`benches/index_accel_results.json`'s two `wasm_bytes`) / ~3.0% off the native
server — the native figure a one-off measurement, not tracked in a results artifact. **This is a *size-floor* lever, not a fixture-speed
one:** `deno task bench:index-accel` measures acceleration ~1.8–2.0× faster on
reads from ~200 rows up, so the fixture/test audience wants it **ON** — the
stripped build is for a genuinely size-constrained consumer that accepts slower
reads. (It is the first compile-time layer. The feature-resolution
mechanics — how the flag actually rides the dependency graph, and how to read
it off `cargo tree` — are `TRADEOFFS.md` § Modularity's.)

```bash
# stripped wasm
wasm-pack build crates/pglet_wasm --target web -- \
  --no-default-features --features console_error_panic_hook
# stripped native server (file_store rides the dep lines, so only acceleration drops)
cargo build --release -p pglet_server --no-default-features
# stripped regression lock — the `@profile-lock` subset (chained into `deno task test`)
deno task test:fixtures:profile-lock
# full stripped differential run — the harness builds the spawned server --no-default-features
PGLET_NO_INDEX_ACCEL=1 deno task test:fixtures
```

### The profile lock

**The byte-identical *intent*'s guard is
`deno task test:fixtures:profile-lock`.** It runs the differential harness
under `PGLET_NO_INDEX_ACCEL=1` (so the spawned server is the stripped build)
over the fixtures tagged `-- @profile-lock` — the subset whose `@test` takes a
*different plan* on the two profiles. It is chained into `test`, right after
`test:fixtures`. It is deliberately **not** in `check`: `check` is a hermetic
lint/format/release-audit gate runnable anywhere, while the fixture harness
needs a live Postgres for its text-format leg (`127.0.0.1:5432` unless
`PGLET_PG_URL` names another), and giving
`check` a Postgres dependency is a larger contract change than this guard
warrants.

**The stripped build compiles under `check`.** `deno task check:stripped`
(`cargo check --workspace --all-targets --no-default-features`, chained into
`check`) keeps the `#[cfg(feature = "index_acceleration")]` gates honest:
without it, a new planner test naming `Plan::IndexScan` breaks the stripped
build with no signal until someone remembers to compile it. Compile-only and
hermetic — no server, no Postgres — so `check` stays runnable anywhere. It
costs ~6 s on a cold target dir, ~2 s after a source edit, and nothing when
warm.

**The two profiles are held to different bars, and the word is `check` vs
`clippy`.** The default profile runs
`cargo clippy --workspace --all-targets -- -D warnings`, so a rustc warning and
a clippy lint both fail `check`. The stripped profile runs plain `cargo check`:
a compile **error** fails, a rustc warning does not, and no clippy lint is
computed at all. So a lint that only fires under `--no-default-features` — a
now-dead `use`, an unread field behind the `#[cfg]` — is invisible until
someone runs clippy on that profile by hand. The one exception is
`check:wasm-features`, which runs clippy over `pglet` and `pglet_wasm` alone, for
the wasm target, under the stripped build's feature set as well as the default's (and
`pglet_wasm`'s `opfs` one);
`pglet_sql`, where most of the `#[cfg]` gates live, is not among them.

Some of the planner tests that survive the stripped build are
**vacuous by construction** — `order_by_nullable_indexed_col_keeps_sort`,
`order_by_non_indexed_col_keeps_sort`, `column_equals_column_keeps_filter_scan`,
`partial_index_range_keeps_filter_scan`,
`partial_gin_unused_when_query_does_not_imply_predicate`,
`gin_probe_may_be_a_correlated_outer_ref_but_not_an_inner_column`,
`order_by_gin_indexed_col_keeps_sort`, and
`equality_and_range_on_gin_indexed_col_keep_filter_scan` all assert that the
planner *declines* an index, and the stripped planner always declines. Count
the real stripped planner coverage accordingly.

**The tag set cannot rot.** In `--profile-lock` mode the harness runs two
completeness checks before the fixtures. First, it plans every fixture's
`@test` in-process (it is itself built with acceleration on, whatever profile
the spawned server uses) and **fails if any fixture that plans to an
`IndexScan` / `IndexRangeScan` / `GinScan` is missing the directive**. Second —
because index selection runs *inside* the planner, so a query that errors
during it yields no plan and the first check reads it as "not accelerated" — it
**fails any `@expect-error` fixture whose `@setup` builds an index** unless the
fixture carries `-- @profile-lock` or a justified `-- @no-profile-lock: <why>`.
Over-tagging is fine; the second check is a crude heuristic whose job is to
convert a silent omission into a forced decision, not to prove anything
(`tests/README.md` § Profile lock names the residual hole). A `-- @…` comment
line naming an unknown directive is a hard parse error, so a misspelled
`-- @profile-lokc` fails the run instead of silently dropping the fixture from
the set.

**What the guard is worth, measured.** The
main `test:fixtures` run already compares the *accelerated* profile against
Postgres for every fixture, and every lock fixture is also in that run. So the
lock's marginal detection is confined to code the *accelerated* plan of a lock
fixture bypasses and no other fixture reaches — a genuinely narrow surface,
since the untagged fixtures exercise the row-filter engine heavily.

A bounded mutation sweep over that surface found no such bug. Each mutation measured
aimed at the paths the accelerated plan skips — per-type arms of
`values_equal`/`values_compare` (interval, mixed time/interval, timestamp, text
ordering, uuid), comparison-operator semantics (`<` as `<=`, `>=` as `>`),
`Filter(Scan)` NULL handling, the shared `ORDER BY` comparator's NULL placement, array
`@>` containment, and `LIMIT`/`OFFSET` — **reddens the main run**, every lock-run red
among them included, and the mutations that look lock-unique are not: reverting the
comparison-operand gate (`comparison_legality` in `planner/legality.rs`) and dropping
the `ORDER BY` comparator's `.reverse()` each redden the main run (`select/order_by*` for the
second).
No mutation is known that this guard catches and the main run misses.

So its value: the lock run is a **cheap confirmation that the
stripped profile still agrees with Postgres on the divergence-capable subset**
— a fraction of the main run's time, the tagged subset being a fraction of the
corpus — kept for defense in depth. (The run prints the count, and the gate
runner's summary the timing.) Its
*marginal* detection over the main run is unproven. Two corollaries:

- **Do not read a green lock run as "the index paths are fine."** A bug
  confined to the index paths (a lossy probe coercion, a bad index-key order)
  never reaches the stripped build. Dropping the exact-or-decline probe rule
  turns `types/compare_int_numeric_literal_indexed.sql` red in the *main* run
  and leaves the lock run green. That direction is the main run's job.
- **Do not read a green lock run as "the stripped profile is fine" either.** It
  covers the tagged subset only; the full stripped run is the thorough check.

A quick sanity check that the knob is actually wired: the spawned
`target/debug/pglet_server` differs between the two runs (compare size or
hash) — a silently-ignored knob reports green against the accelerated binary
twice.

A **full** stripped run (`PGLET_NO_INDEX_ACCEL=1 deno task test:fixtures`, all
fixtures) is still the thorough check, and is worth running by hand whenever a
change touches the planner's index selection, the `IndexScan` /
`IndexRangeScan` / `GinScan` read paths, or any operand/type gate that one
profile can reach and the other cannot. The lock is the automated floor beneath
it, not a replacement.
