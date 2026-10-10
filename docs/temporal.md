# Temporal types — DATE, TIME, INTERVAL, timestamps

The temporal half of the type surface, in full. The summary lives in the repo
`CLAUDE.md` § Type system; NUMERIC has its own doc ([numeric.md](numeric.md)),
and `types/interval.rs` carries the same grain of detail inline for INTERVAL's
internals. pglet is UTC-only throughout — that constraint shapes every
divergence noted below.

## The date and timestamp domain

**A finite `date`, `timestamp` or `timestamptz` value is inside `0001-01-01` through
`9999-12-31` — a `timestamptz` read in UTC — and a finite value outside it is refused, never
produced.** `infinity` and `-infinity` are not finite values and are not refused
(§ `infinity` and `-infinity`). 18.6's own ranges are far wider (a date from 4714-11-24 BC to
5874897-12-31, a timestamp to 294276-12-31) and it prints the years before 1 with a
` BC` suffix. pglet models no BC, and its values are `time`'s `Date` / `OffsetDateTime`,
whose years run from -9999 to 9999. A year 0 or a negative year is representable there
and the text encoder prints it (`0000-12-31`), which no Postgres input function reads back:
a table holding one dumps to a `COPY` 18.6 refuses, and `psql -f` exits `0` with the
table empty.

The predicate is `pglet_core::types::check_date_domain` /
`check_timestamp_domain`, the second judging an instant in UTC and handing it back at
UTC, and these are the places that call it:

| producer or door | where | answer past an edge |
| --- | --- | --- |
| text input — `parse_date`, `parse_timestamp`, `parse_timestamp_naive` (a cast from text, a typed constant, a `COPY FROM` field, a text-format parameter, an array literal's element and a value the wasm host binds each reach one) | `types/text.rs` | for a timestamp, each field's own verdict where 18.6's field loop raises it (§ Reading a timestamp spelling): a numeric UTC offset outside 18.6's ranges is `22009 time zone displacement out of range: "<input>"`, and a time of day out of range is `22008 date/time field value out of range: "<input>"`; after the loop the date's fields **as written** — a year 0, a month or day out of range — are that same `22008`, before an offset or a rounding moves the instant; an offset, a sub-second rounding, a seconds field of 60 or an hour of 24 carrying the UTC reading across either edge is `22008 timestamp out of range`. A spelling the reader refuses as malformed is `22P02` for a timestamp (`EDGES.md` row 96). A date's year past 9999 is `22008 date out of range` |
| binary input — the `date`, `timestamp` and `timestamptz` receive arms | `types/binary.rs` | `22008 date out of range` / `timestamp out of range`, at every count outside the domain other than the two sentinels each type reads as `infinity` / `-infinity` |
| the microsecond rounding of a text fraction, and a seconds field of 60 or an hour of 24 | `decode_timestamp_fields`, the reader's check following it | `22008 timestamp out of range` for a carry into year 10000 |
| `date ± integer` | `date_shift` | `22008 date out of range` |
| `date ± interval`, `timestamp ± interval`, `timestamptz ± interval` | `timestamp_add_interval` | `22008 timestamp out of range`, judged on the result, and on the months and days passes against 18.6's range (below) |
| `date + time` | `eval_arithmetic` | `22008 timestamp out of range` |
| `date_trunc` | `date_trunc_timestamp` | `22008 timestamp out of range` — reachable from inside the domain: `decade` over years 1–9 is year 0 |
| a `(p)` sub-second rounding | `round_timestamp_to_precision` | `22008 timestamp out of range` (§ Sub-second precision) |
| `now()`, `CURRENT_TIMESTAMP`, `CURRENT_DATE`, `LOCALTIMESTAMP`, the other clock reads | `builtins::wall_clock` | `22008 timestamp out of range` for a clock before year 1; on wasm a JS clock past year 9999 is an instant `OffsetDateTime` cannot hold and is `22000` (§ The transaction and statement clocks) |
| the `date → timestamp` / `timestamptz` and `timestamp → date` casts | `executor::coerce` | `22008 date out of range` / `timestamp out of range`; the `→ date` half reads the instant's UTC date |
| a `Value` bound through `Database::execute_bound` | `normalize_value_domain`, after `bind` | the element's own refusal; a `timestamp` bound at another offset is brought to UTC, and one carrying nanoseconds below the microsecond is truncated toward the earlier instant (below) |
| a value a write stores, and the key an `ON UPDATE CASCADE` converts for a referencing column whether or not a child row takes it | `coerce_column_value`, last (`normalize_stored_value_domain`) — for the action's key the whole of it, the `(p)` rounding included, from `executor::dml`'s `action_assigned_values`, once per firing and ahead of the lookup of the rows the action rewrites | the element's own refusal; a `timestamp` is stored at UTC and a whole microsecond. Only the columns a write assigns reach it: an `UPDATE` of another column rewrites the row with its other values as stored. The action's key is refused with no child row to store it: a `timestamp(0)` or `timestamptz(1)` child of a key moved to a value its rounding carries into year 10000 is `22008 timestamp out of range` where 18.6's action matches no row and the `UPDATE` stands (`EDGES.md` row 142) |
| the dump's stored rows — not its stored expression text | `dump::temporal_restore_failure`, asked by `refuse_unreadable_values`'s row walk | the export refuses: `22008 date out of range` / `timestamp out of range` for a value whose UTC reading or rendered fields are outside the domain, and `22000 cannot dump a timestamp stored at UTC offset …` for one at another offset, whose text would reload as a different instant |

**UTC is half of the domain, not a detail of it.** Comparison, ordering and the
`→ date` cast read an instant in UTC, but the text renderers print its fields at the
offset it carries, and `+ interval`, `extract` and `date_trunc` take their calendar fields
from that offset too. The producers in the table build their instants at UTC, so the
readings agree there; a caller of `execute_bound` can hand the engine one at another
offset — `0000-12-31 22:00:00-05`, which is `0001-01-01 03:00:00` in UTC — and that door,
like the store, brings the value to UTC rather than only judging it. `GREATEST`/`LEAST`, `MIN`/`MAX` and `COALESCE` select a value
rather than build one and carry no check.

**A value that arrives already built** is made a microsecond value at the same two doors.
18.6 holds no precision below the microsecond, and pglet's store persists the binary
format's microsecond count, so a `timestamp` bound with nanoseconds — what
`OffsetDateTime::now_utc()` builds — would otherwise compare by them in memory and not
after the store is reopened: untruncated, over a file-backed store,
`ts = timestamp '2026-01-01 00:00:00'` against a bound `2026-01-01 00:00:00.000000999`
answers no rows and then one, and a bound `1969-12-31 23:59:59.9999995` renders
`…23:59:59.999999` and then `1970-01-01 00:00:00` (measured). The doors **truncate toward the
earlier
instant** — `now()`'s rule, not text input's: 18.6's `'1969-12-31 23:59:59.9999995'::timestamp`
rounds to `1970-01-01 00:00:00`, measured, while pglet's embed door answers
`1969-12-31 23:59:59.999999`. That is a choice, not a measurement of 18.6, which has no
door taking an already-built instant with nanoseconds (its binary format carries none).
The reasons: the value an embedding caller most plausibly binds with nanoseconds is a clock
read, and truncating one gives the instant `now()` would; and the text renderer already
prints an instant's microsecond field that way, so no value renders differently for it.
`crates/pglet/tests/temporal_infinity.rs` guards it across a reopen.

**Where 18.6 also refuses, the answer is 18.6's at the members measured** — its code and
its sentence: an offset 18.6 refuses beside a year-0 field or carrying the instant past an
edge (`'0000-12-31 23:00:00-16'::timestamptz`, `'0001-01-01T00:00:00+23:59'::timestamptz`,
both `22009`), a signed year whose digits 18.6 reads as an offset out of range
(`'+9999-12-31 23:00:00-05'::timestamptz`, `22009`), a year-0 field, `'0001-01-01'::date - 2147483647`,
`'9999-12-31'::date + 2147483647`, `timestamp '0001-01-01' - interval '10000 years'`
and its `date` twin, and `timestamp '2000-01-01' - interval '-2147483648 days'`
(`22008 interval out of range`). It is **not** 18.6's at these, each measured:

| statement | 18.6 | pglet |
| --- | --- | --- |
| `'+0000-12-31 12:00:00'::timestamp` | `22007 invalid input syntax` | `22P02` |
| `timestamp '2000-01-01' - interval '-2147483647 mons -2147483647 days -2562047788:00:54.775807'`, and its `date` and `timestamptz` twins | `22008 interval out of range` | `22008 timestamp out of range` |

The first reads its `+0000-12-31` as an offset whose hour is in range and whose field is
malformed, row 96's code; the last negates to the field triple
18.6 reads as `infinity`, which its negation refuses and pglet's, having no infinite
interval, does not (`EDGES.md` row 143).

**Where 18.6 answers a value — a `BC` result, or a year past 9999 — pglet's refusal is its
own**: `22008`, with the bare `date out of range` / `timestamp out of range` 18.6 itself
spends when a value leaves its range, short of a timestamp literal the grammar refuses
first (`22P02`, the table's first row). So `'9999-12-31'::date + 1` is `22008`, the code
`date_pli` itself uses where 18.6's range ends, and not the `22003` an integer overflow
answers. Those refusals are `EDGES.md` row 142, together with what they cost a binary
parameter.

**`± interval` judges its months and days passes against 18.6's range, and the result
against the domain.** `timestamp_add_interval` runs the months, days and microseconds
passes over integers, refusing a months or days pass outside Julian day 0 (`4714-11-24 BC`)
through `294276-12-31`. The microseconds pass has no range check of its own: its `i64`
overflow check and the domain check on the result stand in for one. So
`date '9999-12-31' + interval '1 day -1 microsecond'` is `9999-12-31 23:59:59.999999`
on both engines.

**The store's finalizer walks an array only when its declared element type is temporal**
(`normalize_stored_value_domain`), and that gate rests on where the finalizer's arrays
come from — reasoned from the producers, not enforced by the type and not measured: the
readers and the array constructor build an array's elements at its declared element type,
and `Database::execute_bound`, where a caller can hand over an array whose declared element
type does not match its elements, walks every element before any write sees it. A store
written without the domain could hold such an array, and the gate would let a write copy it
— but such a store is older than `PGMO`, which reopen refuses, and the dump still refuses
the copy.

**A store written without the domain is not opened, and its rows would still be refused.**
Every such store carries a store magic older than `PGMO`, which reopen refuses
([storage.md](storage.md) § Store versioning); the guards below stand for a value put into
storage directly, which is how `crates/pglet/tests/temporal_domain.rs` holds them. Measured
on a store holding a year-0 `date`, `timestamp` and `timestamptz`: each renders
as stored, and the `date` casts to `text` and compares; a cast that builds a new value
from one (`d::timestamp`, `t::date`, `tz::date`) is refused, and so is a write that would
store one — `INSERT … SELECT` of the value, and an `UPDATE` assigning it back — while an
`UPDATE` that does not assign the column rewrites the row with the value as stored. The
dump refuses such a row with the value's `22008` rather than emit the `COPY` line 18.6
refuses. Deleting the row lets it export; assigning one such column an in-domain value
leaves the others to refuse (measured: the `date` assigned, the export then refused at the
`timestamp`). No migration rewrites such a store.

**The refusal is the rows', not the catalog's.** The dump reads a stored expression's
text as it was written and does not ask it. This build refuses an out-of-domain constant
in a `DEFAULT` (`22008`, the plan-time constant check), but accepts one in a `CHECK` or a
partial-index predicate over an **empty** table — 18.6 refuses both `22008` — which is the
stored-expression constant-cast member of `EDGES.md` row 51. What each shape loses is
measured on stores an older build wrote, older than `PGMO` and so not opened by this one,
each shape in its own store: `DEFAULT '0000-12-31'` on a `date` or `timestamp` column,
`CHECK (d <> date '0000-12-31')`, or a partial index `WHERE d > date '0000-12-31'` exports
at exit `0`, and loading that dump into 18.6 with `psql -f` also exits `0`, short. The
`DEFAULT`'s `CREATE TABLE` is refused, so that table and its rows are lost — and its
`COPY` block, sent to a table that does not exist, leaves psql reading the rows as SQL,
which loses the rows of the tables dumped after it too; the `CHECK` and the partial index
are each refused alone, their tables and rows loading without them.

## `infinity` and `-infinity`

**Each of the three types holds `infinity` and `-infinity` beside its calendar values, as
their own variants and never as a calendar value that renders as the word.** `Value::Date`
and `Value::Timestamp` carry `pglet_core::types::Infinite` (`NegInfinity`, `Finite`,
`Infinity`), so a site that reads calendar fields has to take the `Finite` arm by name. The
wire, the store and the index key all carry 18.6's binary count — days or microseconds
since `2000-01-01`, with `i32::MIN` / `i32::MAX` (`DATE_NOBEGIN` / `DATE_NOEND`) and
`i64::MIN` / `i64::MAX` (`TIMESTAMP_NOBEGIN` / `TIMESTAMP_NOEND`) for the infinities — so
the key order is the value order; the store magic moved to `PGMO` with that encoding
([storage.md](storage.md) § Store versioning).

**Spelling.** The text readers take the word in any case, with an optional `+` or `-` and
whitespace (C's `isspace`) around the word and between sign and word — `' - Infinity '` —
and render `infinity` / `-infinity`, a `timestamptz` with no `+00`. That is narrower than
18.6's tokenizer, and the difference is a property of it rather than a list of spellings:
`ParseDateTime` skips an ASCII punctuation byte (`ispunct`) as a field delimiter wherever it
gives that byte no role of its own, and it reads the word as one field among others, so
the word beside such a byte, or beside an `AD` / `BC` field, is still `infinity` there.
Measured, each accepted by 18.6 and refused here — `22007` for a
`date`, `22P02` for a timestamp — at all three types: `'infinity;'`, `'(infinity)'`,
`'infinity:'`, `',infinity'`, `'infinity,'`, `'!infinity'`, `'infinity@'`, `'infinity_'`,
`'{infinity}'`, `'"infinity"'`, `'infinity ""'`, `'infinity BC'` and `'-infinity ad'`. The
bytes the tokenizer does give a role — `.`, `/`, `-` and `+` directly after the word — are
refused by both (`'infinity.'`, `'infinity/'`, `'infinity-'`, `'infinity+'`, measured, with
row 96's `22P02` against 18.6's `22007` for a timestamp), and so are `'infinity 1'`,
`'infinity today'` and `'infinity utc'`.

**What each operation answers**, every row measured on both engines at the operand types
it names:

| operation | answer, both engines |
| --- | --- |
| order, `=`, `min`/`max`, `greatest`/`least`, `GROUP BY`, `DISTINCT`, an index | `-infinity` before every value, `infinity` after; a `date` infinity equals the timestamp infinity of the same sign |
| `date ± integer`, `date + time`, `date_trunc`, a `(p)` modifier, the `date`/`timestamp`/`timestamptz` casts | the same infinity, after the operation's other checks |
| `date`, `timestamp` or `timestamptz` `± interval`, the interval not one of the field triples 18.6 reads as an infinite interval (below) | the same infinity, after the operation's other checks — `timestamp 'infinity' - interval '-2147483648 days'` is still `22008 interval out of range` |
| `timestamp` or `timestamptz` `AT TIME ZONE 'UTC'` | the same infinity |
| `timestamp::time`, `timestamptz::time` | NULL |
| `to_char` of a `timestamp` or `timestamptz` | NULL, whatever the format |
| `isfinite(date \| timestamp \| timestamptz)` | `false` |
| `date_part` of `year`, `decade`, `century`, `millennium`, `julian`, `isoyear`, `epoch` | `Infinity` / `-Infinity`; every other recognized field NULL |
| `EXTRACT` of a cycling or zone field | NULL — except `EXTRACT(… FROM date)` of a time-of-day or zone field, `0A000` as for a finite date |
| `date - date` with an infinite operand | `22008 cannot subtract infinite dates` |
| an infinity less the same infinity | `22008 interval out of range` |

**Where the two part.** Two answers 18.6 gives are refused loud instead, each `0A000`,
because the type 18.6 answers in has no infinity here: a timestamp difference with one
infinite operand, or two of opposite signs — an infinite `interval` there (`not
implemented: infinite interval`; `PgInterval` has no infinity, `EDGES.md` row 143) — and
`EXTRACT` of a field `date_part` answers `±Infinity` for, a `numeric` `Infinity` there
(`not implemented: numeric Infinity`; `PgNumeric` has none). A third answer needs no
missing type and is refused all the same: `date_part`'s `float8` `Infinity` cast to
`numeric` (`date_part('year', date 'infinity')::numeric`) is `22003 numeric out of range`
here and `Infinity` there — the `float8 → numeric` cast refusing a non-finite value,
whatever produced it. And these parts, each measured:

| statement | 18.6 | pglet |
| --- | --- | --- |
| `date 'infinity' - <max>`, `timestamp 'infinity' - <max>`, `timestamptz 'infinity' - <max>`, `date '-infinity' + <max>`, `timestamp '-infinity' + <max>`, `timestamptz '-infinity' + <max>` | `22008 timestamp out of range` | the infinity unchanged |
| `date 'infinity' + <min>`, `timestamp 'infinity' + <min>`, `timestamptz 'infinity' + <min>` | `22008 timestamp out of range` | the infinity unchanged |
| `date 'infinity' - <min>`, `timestamp 'infinity' - <min>`, `timestamptz 'infinity' - <min>` | `infinity` | `22008 interval out of range` |
| `date '-infinity' - <min>`, `timestamp '-infinity' - <min>`, `timestamptz '-infinity' - <min>` | `22008 timestamp out of range` | `22008 interval out of range` |
| `date '2000-01-01' + <max>`, `timestamp '2000-01-01' + <max>`, `timestamptz '2000-01-01' + <max>` | `infinity` | `22008 timestamp out of range` |
| `date '2000-01-01' - <max>`, `timestamp '2000-01-01' - <max>`, `timestamptz '2000-01-01' - <max>`, and the same three `+ <min>` | `-infinity` | `22008 timestamp out of range` |
| `date '2000-01-01' - <min>`, `timestamp '2000-01-01' - <min>`, `timestamptz '2000-01-01' - <min>` | `infinity` | `22008 interval out of range` |
| `date 'infinity' AT TIME ZONE 'UTC'` | `infinity` | `0A000`, as for a finite `date` |
| `to_char(date 'infinity', 'YYYY-MM-DD')` | NULL | `0A000`, as for a finite `date` |
| `timestamp 'infinity' AT TIME ZONE '+05'` | `infinity` | `0A000`, as for a finite value in a zone other than UTC |
| `isfinite(interval '1 day')` | `t` | `42883` — the `interval` overload is not modelled |
| `isfinite(time '01:00')` | `t`, through `time`'s implicit cast to `interval` | `42883` |
| `sum(tm)`, `avg(tm)` over a `time` column | the sum or average as an `interval`, through the same cast; no row where no group forms | `42883`, a statement that forms no group included — `EDGES.md` row 307 |
| `isfinite(NULL)`, `isfinite(NULL::text)`, and `isfinite(i)` over an `int` column holding one NULL, or over an empty table | `42725` for the first, `42883` for the others | NULL, or no rows — `EDGES.md` row 144 |

`<max>` is `interval '2147483647 mons 2147483647 days 2562047788:00:54.775807'` and `<min>`
is `interval '-2147483648 mons -2147483648 days -9223372036854775808 microseconds'`, the
field triples 18.6 reads as `infinity` / `-infinity` and pglet as finite intervals
(`EDGES.md` row 143). Added to `infinity` or subtracted from `-infinity`, `<max>` leaves the
infinity unchanged on both engines, and so does `<min>` added to `-infinity` — each measured
at the three types.

## DATE

A full column type (`Value::Date`, `YYYY-MM-DD` text /
i32-days-from-2000-01-01 binary, B-tree-indexable, comparable, casts text↔date
and `timestamp::date`). Arithmetic is complete: `date ± int → date`,
`date - date → int4`, `date ± interval → timestamp` (in `eval_arithmetic`, with
result-OID inference in `temporal_arith_type`). Text input accepts
1-or-2-digit month/day (`2026-1-1`, like Postgres); a malformed spelling →
`22007` (`invalid_datetime_format`), a shaped-but-out-of-range field
(`2026-13-01`, `2026-02-30`) → `22008` (`datetime_field_overflow`). A
leading-`-` year is rejected loud (pglet models no BC): `22007`, which is PG's code
for `'-0001-01-01'` too, and a **divergence** where PG reads the sign as a time zone
displacement and answers `22009` (`'-2026-06-15'`).

`CURRENT_DATE`/`CURRENT_TIMESTAMP` are Postgres **SQL value functions**
(`SqlValueFunction`) — parenless special forms recognized in the parser, not
functions, so `current_timestamp()` fails loud (`42601`) like Postgres, at
every position including a column `DEFAULT` (though Postgres does accept the
precision form `CURRENT_TIMESTAMP(p)`, which pglet rejects along with any other
parens there — this is the *value function's* grammar and not the type's, which does
model a sub-second precision: § Sub-second precision); `CURRENT_TIME`/`LOCALTIME` are wired (returning
plain `time` — see § TIME), and `LOCALTIMESTAMP` is wired (the `now()` instant
typed `timestamp`/no-tz, so its text has no `+00` offset). The non-temporal
value functions — `CURRENT_USER`, `SESSION_USER`, `USER`, `CURRENT_ROLE`,
`CURRENT_CATALOG`, `CURRENT_SCHEMA` — are modeled as special forms too
([compatibility.md](compatibility.md)'s *Session information functions* row), which is why
`SqlValueFunction` has a `takes_no_argument_list` split: `current_schema` is the
one member 18.6 also carries as a `pg_proc` entry, so a `(` after it is a call
rather than the `42601` every other member owes. `DEFAULT CURRENT_DATE` is wired, and
`DEFAULT LOCALTIMESTAMP`/`LOCALTIME` are accepted through the
`ColumnDefault::Expr` catch-all rather than a closed arm.

### Reading a timestamp spelling

**A timestamp spelling is split into fields as 18.6's `ParseDateTime` splits it, and the
fields are read as its `DecodeDateTime` reads them** — `split_datetime_fields` and
`decode_timestamp_fields` in `types/text.rs`, for the field kinds pglet models.
Whitespace separates fields and so does any other ASCII punctuation byte the tokenizer
gives no role, so `2000-01-01 12:34:56`, `2000-01-01/12:34:56`, `2000-01-01_12:34:56`,
`2000-01-01:12:34:56` and `2000-01-01#12:34:56` are one value on both engines. The bytes
that do have a role are the date's `-`, `/` and `.` delimiters, a time's `:` and `.`, and
a sign: `2000-01-01-12:34:56` and `2000-01-01.12:34:56` run the time into the date field
and are malformed, and in `2000-01-01+12:34:56` the `+` starts an offset, so the value is
midnight at `+12:34:56` (`1999-12-31 11:25:04+00`). A letter starts a word; `t` in either
case between the date and the time is 18.6's ISO filler. A word that runs straight into a
`-`, `/` or `.` runs on into one field read as a zone name (below), and so does a word
running straight into a digit or a `+` — unless the word is one of 18.6's own date tokens
(`datetktbl`: `t`, a month or day name, `am`, `ad`, …), which stays a word: `abc5`,
`abc+5`, `july-5` and `utc+16` after a date are zone names on both engines, while `t5`,
`t+05` and `july5` are refused on both. A control byte or a non-ASCII byte is malformed.
All measured.

The fields modelled, each at most once and **in any order**, save that a date after a
time of day is malformed (`00:00:00 2000-01-01`, as `DecodeDate` refuses it): a
`y-m-d` date whose year has at least three digits (`200-01-01` is year 200; `2000/01/01`,
`2000.01.01`, `2000-01-001` and a trailing `2000-01-01-` read the same), a time of day, a
numeric offset, the word `z`, a POSIX zone after the date, and the `t` filler after the
date.

**A time of day is read as `DecodeTime` reads it.** `hh:mm`, `hh:mm:ss`, and — when a
fraction follows two fields — `mm:ss.f` (`12:30.5` is `00:12:30.5`); an empty field is `0`
(`12::30` is `12:00:30`, `12:` is noon); each field is ranged on its own — a minute to
`59`, a **second to `60`** — and then `time_overflows` takes the whole time of day to at
most `24:00:00`. So a seconds field of 60 is accepted **at any time of day**, not only at
a month's end, and carries into the next minute (`2000-06-29 23:59:60` is
`2000-06-30 00:00:00`); a fraction on top of it is accepted where the total stays within
the day (`12:30:60.5` is `12:31:00.5`) and refused where it does not
(`'2000-06-30 23:59:60.5'` and `'2000-01-01 24:00:00.5'` are
`22008 date/time field value out of range: "<input>"` on both). An hour of 24 is exactly
`24:00:00`, the next day's midnight. A carry out of the date domain is the domain's
refusal: `'9999-12-31 23:59:60'` is `10000-01-01 00:00:00` on 18.6 and
`22008 timestamp out of range` here (`EDGES.md` row 142).

**A fraction is `rint(strtod(fraction) * 1000000)` over a C `double`**, the tie going to
the even microsecond — but it is the `double` that ties, not the decimal: `.0001255` is
`.000125` and `.0001265` is `.000127` on 18.6 (neither is a tie once it is a `double`),
while `.0000015` and `.0000025` are both `.000002`. `parse_frac_micros` is that rule for
the timestamp readers and for `time` alike. Measured at both types.

**The codes follow 18.6's field loop.** A time of day out of range answers where 18.6's
loop reaches it, so it outranks an out-of-range offset written after it
(`'2000-01-01 25:00:00+16'` is `22008` on both) and loses to one written before it; the
date's own fields — a year 0, a month outside `1 ..= 12`, a day outside `1 ..= 31` or past
its month's end — are judged after the loop, `22008 date/time field value out of range`
on both (18.6 adds a `DateStyle` hint to the month and day-range sentence, which pglet
does not send). A spelling the reader refuses as malformed is `22P02 invalid input syntax
for type timestamp` here against 18.6's `22007` — `EDGES.md` row 96, measured on its five
routes.

**A numeric UTC offset is read as 18.6's `DecodeTimezone` reads it.** A sign, whitespace
skipped after it, then a digit and every following digit, `:`, `.` and `-`: an hour, then
an optional `:minute` and `:second` — or, with no `:` and more than two digits, a
run-together `hhmm` (`+0530`; `+155` is `01:55`). Each field is read by `strtoint`, so an
empty one is `0`. The hour is `0 ..= 15` and the minute and second `0 ..= 59`; outside
them, or past `int`, the literal is `22009 time zone displacement out of range: "<input>"`
— on `timestamp` too, which drops the offset only once it has read it, and at a column
`DEFAULT`, refused at `CREATE TABLE`. The range checks answer before the check that the
field was read whole (`+16.5` is `22009`, `+05.5` malformed: row 96's `22P02` here, `22007`
there). An offset is a field of its own wherever it stands: after an empty fraction
(`'2000-01-01 00:00:00.+16'`), as a second offset (`'…+05+16'`, whose range is judged
before it is found to be a second one), before the date (`'+05 2000-01-01 00:00:00'`), and in a
signed year, whose digits are an offset (`'+2026-06-15 12:00:00'` is `22009` on both,
`'+0000-12-31 12:00:00'` malformed). All measured.

**A zone name is read only as a POSIX zone with no daylight-saving part** —
`posix_zone_seconds`: a name, then `[+|-]hh[:mm[:ss]]` with the hour at most `167`, the
minute `59` and the second `60`, ending the field, the offset counting **west**
(`'2000-01-01 00:00:00 utc+16'::timestamptz` is `2000-01-01 16:00:00+00`, and
`2000-01-01T00:00:00Z+05` reads `z+05`). The word `z` is UTC. Everything else a zone name
can be is refused as malformed here where 18.6 reads it — a named zone or abbreviation
(`EST`, `UTC`, `America/New_York`), and a POSIX zone with a daylight-saving part, which
18.6 resolves against default rules, so `'2000-06-30 00:00:00 x5z'::timestamptz` is
`04:00:00+00` in June there. A letter written where a separator would be is read the
same way: `'2000-01-01x12:34:56'` is the POSIX zone `x12:34:56` on both engines —
`12:34:56+00` as a `timestamptz`, and the date's midnight as a `timestamp`, which drops the
zone — while `'2000-01-01x12:34:56.5'` is a value there, read as a zone with a
daylight-saving part (`x12:34:56`, then `.5`), and malformed here,
and `'2000-01-01x12:34:56+05'` is `22023 time zone "x12:34:56+05" not recognized` there and
malformed here. Each refusal here is `22P02`, so none of these is silent.

**What else 18.6 reads and pglet refuses as malformed**, each measured as a `timestamp`: a
month name (`Jan 1 2000`), a year and a day of year (`2000-060` and `2000.060` are
`2000-02-29 00:00:00` there, while `2000-060-01` is `22007` on 18.6 too), a year of one or
two digits (read in `DateStyle` order there: `01-02-03` is `2003-01-02`), a run-together
`hhmmss` (`2000-01-01 123456`), `am`/`pm` (`2000-01-01 12:00 pm`), the `at`/`on` noise words
(`2000-01-01 at 12:00`), the special words `epoch`, `now`, `today`, `tomorrow` and
`yesterday`, and `allballs` after a date (`2000-01-01 allballs` is midnight there; a bare
`allballs` is `22007` on 18.6 as a `timestamp` or a `timestamptz`). The
`date` and `time` readers are separate and narrower still: they refuse an offset after a
time of day, `22007` (`'00:00:00+05'::time`, `'2000-01-01 00:00:00+05'::date`), where 18.6
reads the offset and drops it, and the `date` reader refuses a trailing time of day
(`'2024-01-01 00:00'::date` is `22007` here and `2024-01-01` there).

## The transaction and statement clocks

**The clock family reads one of three clocks, and which one is the spelling's** — 18.6's
split (`xact.c`'s `xactStartTimestamp` and `stmtStartTimestamp`, and `GetCurrentTimestamp`),
measured on the oracle spelling by spelling:

| clock | spellings | the instant |
| --- | --- | --- |
| the transaction's start | `now()`, `transaction_timestamp()`, `CURRENT_TIMESTAMP`, `LOCALTIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIME` | the start of the statement that began the transaction: an open block's `BEGIN`, and with no block open the statement itself |
| the statement's start | `statement_timestamp()` | when the client's message arrived |
| the wall clock | `clock_timestamp()` | the call |

Two reads of a transaction-start spelling inside one transaction are one value at each
door measured against 18.6 — the select list, a predicate over many rows, a `DEFAULT` on
several columns and many rows, `INSERT … SELECT`, `UPDATE … SET`, `ON CONFLICT DO UPDATE`,
`RETURNING`, a `CHECK`, a CTE and a subquery, the statements of a `DO` body, an `ADD COLUMN`
back-fill, a `COPY`'s defaults, a statement with bound parameters, and an index probe's
constant beside the row filter the stripped build runs in its place (the `clock_*`
fixtures). A savepoint and a `ROLLBACK TO` are inside the transaction and do not move its
start; the transaction after a `COMMIT` or a `ROLLBACK` — an aborted block's included — has
a start of its own.

**Where the instants live** (`pglet_sql::builtins`' module doc is the statement):

- A host marks each statement's arrival with `builtins::start_statement` — the embed
  facade in the entry guard every public entry passes (so the wasm bindings, which run on
  the facade, take it there), the native server in `execute_query_core` and, for a `COPY`,
  `dispatch_copy`. The native hosts read the clock at that call; on wasm the read is a
  call out to the JS host, so it is made where the statement first needs the instant and
  not at all by a statement that reads no clock.
- An open block carries its start in the connection's `SessionSettings`
  (`begin_block` takes the statement start its `BEGIN` ran under; `end_block` drops it
  with the block), so it is per connection, and it is set and dropped where the block's
  isolation level is. `SessionSettings::install_ambient`, which each statement already
  runs, hands it to the evaluator beside the session.
- Nothing is in the plan: a prepared statement executed twice reads two statement starts.

**One message is one statement start.** A simple-query message carrying several statements
— and one `Database::execute` call — runs them under one instant, and a transaction one of
them begins starts at it: `BEGIN; SELECT now() = statement_timestamp()` sent as one message
is `t` on both engines, and `f` sent as two.

**The protocol residual is the extended protocol's** (`EDGES.md` row 292). 18.6 starts the implicit
transaction at the first message after a `Sync` and restamps the statement at each `Bind`
and `Execute`, so outside a block a bound statement's `now()` is earlier than its
`statement_timestamp()`, and the statements of one pipeline share a `now()`. pglet takes
both instants at the `Execute` (the embed and wasm hosts at the call), so the two are
equal there and each statement of a pipeline has its own. `now() <= statement_timestamp()`
holds on both while the clock is not set back inside an open block (below).

**The wasm clock is `Date.now()`'s millisecond, stepped inside its first half.** A read
answers an instant from the start of the millisecond `Date.now()` reports at that read to
499 microseconds past it, so a JS clock that moves, forward or back, is answered as it
reads: a test runner's fake timer pinned to a date is that date to the millisecond, at any
read count. The first read of a millisecond answers its start.

`Date.now()` is whole milliseconds, so a read that finds the read before it in the same
millisecond is put a microsecond past it (`builtins::step_host_clock`): through the first
five hundred reads of a millisecond, two statements in it have ordered, distinct starts, and
a `clock_timestamp()` is later than the start of its statement (past them: the residual,
below). The last read is kept by the wasm module, not by a database, so the order holds
between the databases of one module too, and the steps of a millisecond are spent by the
module's reads together.

**The tie point is 499 microseconds because a column rounds.** `timestamptz(3)` rounds half
away from the value's zero, 2000-01-01 (§ Sub-second precision): `…00.0005` is `…00.001`
after it and `…00` before it, and `…00.000499` is `…00` on both sides and before 1970. So
every instant a read answers, stored at precision 3, 4, 5 or 6 or unmodified, is inside the
millisecond the JS clock reported, and at precision 3 is that millisecond. Precision 0, 1
and 2 round the pinned millisecond itself, as they round any instant; one case shows the
step there — a pinned millisecond before 2000-01-01 that is exactly a half at that
precision, whose first read rounds down with the millisecond and whose later reads round up.

18.6 reads `gettimeofday` and the native hosts the OS clock, with nothing kept between
reads, so a clock set back is answered by both as well. What a set-back does not move is an
instant already taken: an open block's start stays where its `BEGIN` read it, so inside that
block `statement_timestamp()` and `clock_timestamp()` are then earlier than `now()`, and an
instant answered before the set-back can be answered again after it.

**The step's residual is the tie.** The five-hundredth read of one millisecond answers the tie
point, and each read after it in that millisecond answers it again: those reads are equal —
two statements' starts, or a `clock_timestamp()` and its statement's start — and none is
earlier than the one before. 18.6 ties likewise where two reads fall in one microsecond of
its clock. The engine's own readers of the clock — the builtins, the column defaults, a
block's start — take the value and compare nothing against it; a unique key over the value
does, and refuses a tied row (`23505`).

Under a frozen clock that is every read past the five-hundredth, in whichever database of
the module makes it, until a statement reads the clock in another millisecond — a clock put
back to the real one restarts nothing until one does, so tests pinning one instant spend one
five hundred between them.

On the real clock a statement reading the clock makes one read, and one more at each
evaluation of `clock_timestamp()`, and a read ties only past the five-hundredth of its
millisecond — so whether a shape ties turns on how many reads the host makes a millisecond,
and a shape near five hundred ties sooner on a faster or an idler host. One measured host,
idle, through the npm package under Node, as an example: consecutive statements made under
100 reads a millisecond, and 100 000 of them left no start tied with or earlier than the one
before; a `clock_timestamp()` evaluated a row made roughly 850 to 1100, so a statement of
more than five hundred such rows can repeat a value — an
`ADD COLUMN … DEFAULT clock_timestamp()` back-fill of 100 000 rows left about half of its
values distinct (0.47 to 0.57 of them, run to run), and
`INSERT INTO k (ts, v) SELECT clock_timestamp(), x` into a primary key on `ts` was accepted
in each run at 300 rows, `23505` in some runs at 600 and in each from 1000. 18.6 on the same
host repeated more: about a quarter of that back-fill's values distinct, and that `INSERT`
`23505` in each run at every size measured, from 10 rows.

And a step is past the millisecond's start: under a frozen clock the second read is already
later than the JS time bound as a parameter, as an 18.6 timestamp's microseconds are later
than a JS `Date` taken in the same millisecond.

Measured through the wasm binding and the npm package, `Date.now` replaced by a function the
test moves, `P` a pinned millisecond:

| the JS clock | a read answers |
| --- | --- |
| a read under today's clock, then pinned to `2024-01-01T00:00:00Z` | `2024-01-01 00:00:00+00`, in a `DEFAULT now()`, in `CURRENT_DATE`, and in a database made after the pin; the reads after it in that millisecond `…00.000001+00`, `…00.000002+00`, one a read |
| pinned to `2099-01-01T00:00:00Z`, then the real `Date.now` restored | `2099-01-01 00:00:00+00`, then the real clock |
| frozen at `P` across 2500 statements alternating between two databases | the first five hundred starts distinct, in statement order, `P` to `P + 499 µs`; each start after them `P + 499 µs`; none earlier than the one before |
| the same through the npm package, 2400 inserts each followed by a `SELECT now()`, over two instances and a fork | every `now()` read back as a `Date` is `P`; every row's `created_at` is in `[P, P + 1 ms)` |
| frozen at `P`, 1200 rows stamped into `timestamptz(3)`, `timestamp(3)`, `timestamptz(4)`, `timestamptz(5)` and `timestamptz` columns, `P` in 2024, in 1985 and in 1960 | every row of the two precision-3 columns is `P`; every row of the others is in `P`'s millisecond; `now()::timestamptz(3)` past the tie point is `P` |
| frozen at `P`, 1500 rows stamped into `timestamptz(2)`, `(1)` and `(0)` columns | every row is `P` rounded to that precision — except `P` in 1985 exactly on a half (`….005` at precision 2, `….500` at precision 0): the first row rounds down, as `P` does, and the others up |
| frozen at `P`, 1200 rows stamped, `created_at <= $1` with the JS time bound | `1` row — the first |
| frozen, `ts timestamptz PRIMARY KEY DEFAULT now()`, single-row inserts | `23505` at the 501st read of the millisecond; `UNIQUE (owner, created_at)` likewise |
| four runs of 200 inserts, each pinning the same instant, the real `Date.now` restored between them with only a statement that reads no clock | 200, 400, 500 and 500 distinct starts of 200, 400, 600 and 800 rows |
| past the tie point: `clock_timestamp() = now()`, `clock_timestamp() < now()`; inside a block, `statement_timestamp() = now()` | `t`, `f`; `t` |
| past the tie point, pinned again to `P` by a new `Date.now`, and a database made under it | `P + 499 µs` in both |
| then moved forward one millisecond | `P + 1 ms` exactly, the next read `P + 1 ms + 1 µs` |
| then back to `P` | `P` exactly |
| `BEGIN` at `2025-01-02`, then pinned to `2024-01-01` inside the block | `now()` and `CURRENT_DATE` the block's `2025-01-02`; `statement_timestamp()` `2024-01-01 00:00:00+00`, `clock_timestamp()` a microsecond past it; after `COMMIT`, `now()` on `2024-01-01` |
| read at the year 20000, then put back | refused (`22000`), then the clock as it reads |
| `Date.now()` answering `NaN` | refused (`22000`, `Date.now() did not answer a number`) |

## TIME

A full column type (`Value::Time` = µs-since-midnight, a raw `i64` so the
`24:00:00` endpoint is representable — unlike DATE's `time::Date`).
`HH:MM:SS[.ffffff]` text (trailing-zeros trimmed) / `int64` binary (OID 1083,
array 1183) / plain big-endian index key (non-negative, so no sign-flip —
chronological, enforced UNIQUE), full comparison / `ORDER BY` / range. Input
accepts `HH:MM:SS[.f]`, `HH:MM`, `HHMMSS`/`HHMM` (with an optional fractional
suffix), and 12-hour `AM/PM`. Casts text↔time and `timestamp::time` (truncates
to time-of-day). Arithmetic: `time - time → interval`,
`time ± interval → time` (uses only the interval's µs and **wraps mod 24 h** —
days/months ignored, matching `time_pl_interval`), `date + time → timestamp`.
`CURRENT_TIME`/`LOCALTIME` (value functions) and `DEFAULT CURRENT_TIME` return
plain `time` (PG's `CURRENT_TIME` is `timetz`, but pglet is UTC-only).
Differential-tested (`tests/fixtures/types/time_*.sql`). A malformed spelling
fails loud `22007` (`invalid_datetime_format`); a well-formed but out-of-range
field (`25:00:00`, `12:99`) is `22008` (`datetime_field_overflow`).
**Divergences:** a tz suffix on a time literal fails loud (`22007`; PG
accepts-and-drops it); `TIMETZ` (`time with time zone`) stays unimplemented
(`42704`); `CURRENT_TIME(p)` parens rejected (`42601`). The `time` reader is not the
timestamp reader's `DecodeTime` port (§ Reading a timestamp spelling), and diverges from
18.6 at these spellings, each measured as a `'…'::time` cast:

- **A seconds field of 60** is `22008 time out of range` here: `12:30:60` is `12:31:00` on
  18.6, `23:59:60` is `24:00:00`, and `12:30:60.5` is `12:31:00.5`. `23:59:60.5`, whose total
  passes `24:00:00`, is `22008` on both, with different messages (18.6 says
  `date/time field value out of range: "23:59:60.5"`).
- **The `mm:ss.f` reading and an empty field** are `22007` here: `12:30.5` is `00:12:30.5`
  on 18.6, `12::30` is `12:00:30`, and `12:` is `12:00:00`.
- **`allballs`** is `00:00:00` on 18.6 and `22007` here.
- **A trailing `.` after a run-together `hhmmss` is accepted here** — `123456.` is
  `12:34:56`, where 18.6 refuses it `22007`. That one is a **silent over-accept**; the
  members above are loud. `123456` and `1234.5` agree (`12:34:56`, `12:34:00.5`).

## INTERVAL

A full column type over an in-house 3-field `PgInterval { months: i32, days:
i32, micros: i64 }` (`pglet_core`'s `types/interval.rs`, dep-free — Postgres's
own layout; the split stays because a month's day-count varies). Three input
grammars (verbose `1 year 2 mons`/`ago`, SQL-standard `1-2`/`3 4:05:06`,
ISO-8601 `P1Y2M3DT4H5M6S` where an `M` before `T` is months and after `T` is
minutes) with faithful fractional-unit normalization; `postgres`-style output
(signed pluralization `value != 1`, `mon`/`mons`, `+`-force on a positive group
after a negative one, zero → `00:00:00`); a 16-byte binary wire codec
(**time → day → month** order, OID 1186, array 1187); and an order-preserving
index key. **Comparison is Postgres's estimate** —
`micros + (months*30 + days)*USECS_PER_DAY` in i128 — so
`INTERVAL '30 days' = INTERVAL '1 mon'` (a documented quirk), a UNIQUE interval
index dedups them, and `ORDER BY`/`GROUP BY`/`DISTINCT` fold identically
(custom `PartialEq`, mirroring `PgNumeric`'s `1.5 == 1.50`). Arithmetic:
`interval ± interval`, unary `−`, `interval ×/÷ number` (fractions cascade
down, byte-faithful to `interval_mul`/`interval_div`), `timestamp ± interval`
(a 3-pass calendar shift — month with a leap-aware day-of-month clamp, then
whole days, then µs), `date + interval → timestamp`, and
`timestamp - timestamp → interval` (justified via `justify_hours`). There is no
`infinity`: the field triples 18.6 reads as `infinity` / `-infinity` are finite intervals
here at the text inputs and arithmetic measured, and a binary parameter holding one is
refused (`EDGES.md` row 143). Numeric
fields parse exact (i64, not f64; exponent forms rejected). Not DST-aware —
faithful only because pglet is UTC-only (a day is always 24 h). **Fail-loud
gaps:** an `INTERVAL <fields>` **range** spec (`DAY TO SECOND`, `SECOND(0)`, …) — the
fields restriction changes semantics, so pglet rejects it rather than mis-truncate. At a
**column** the refusal is total over the spellings measured — `interval day to second`,
`interval second(0)`, and the single-field `interval year` and `interval second` are
each `42601 syntax error … expected ')'` here against an accepted `CREATE TABLE` there.
At a **cast** it is only as wide as the tokens left unread: `'1 day 2 hours'::interval
day to second` and `'1.5 secs'::interval second(0)` are `42601` here against `1 day
02:00:00` and `00:00:02` there, but a **single-field** spec is swallowed as a column
**alias** — `'1 year 2 mons'::interval year` answers `1 year 2 mons` here against 18.6's
`1 year`, which is a *silent* wrong value — the class `EDGES.md` row 71 records, whose two
`ISNULL` / `NOTNULL` spellings are closed and whose live member this is. Each measured on both engines. A bare `interval(p)` is a
*precision* rather than a range, and is applied — § Sub-second precision. A malformed
spelling →
`22007` (`invalid_datetime_format`); a **parse-time field overflow**
(`interval '999999999999 years'`) → `22015` (`interval_field_overflow`); an
**arithmetic** overflow (`interval × n`, `timestamp ± interval`) → `22008`
(`datetime_field_overflow`).

## Sub-second precision — the `(p)` four of these types carry

**A modifier the type does carry is kept, clamped by the type's own maximum, and
enforced — never discarded.** `timestamp(p)`, `timestamptz(p)`, `time(p)` and
`interval(p)` are the four (`pg_type.typmodin` is non-zero for each; `timetz` would be
a fifth and is `42704` here whatever the modifier). This is the third arm of
[coercion.md](coercion.md) § A canonical type name decomposes once.

**The rounding rule is 18.6's, and its sign is not the wall clock's.**
`AdjustTimestampForTypmod` (`timestamp.c`), `AdjustTimeForTypmod` (`date.c`) and
`AdjustIntervalForTypmod`'s sub-second half are three copies of one arithmetic — add
half a scale, truncate toward zero — and the sign each branches on is the value's
**internal** representation: microseconds since **2000-01-01** for a timestamp, since
midnight for a `time`, the `micros` field for an interval. pglet has that arithmetic
once, in `pglet_core::types::round_micros_to_precision`, with one wrapper per carrier.
Measured on both engines:

| statement | both engines | what a naive half-up on the fraction gives |
| --- | --- | --- |
| `'2020-01-02 03:04:05.6789'::timestamp(3)` | `2020-01-02 03:04:05.679` | same |
| `'1999-12-31 23:59:59.9995'::timestamp(3)` | `1999-12-31 23:59:59.999` | `2000-01-01 00:00:00` |
| `'2000-01-01 00:00:00.0005'::timestamp(3)` | `2000-01-01 00:00:00.001` | same |
| `'1969-07-20 20:17:40.0005'::timestamp(3)` | `1969-07-20 20:17:40` | `…40.001` |

Ties go **away from zero**, not to even — `'00:00:00.0025'::time(3)` is `.003` on both
engines, where half-to-even gives `.002`; `'-0.0015 secs'::interval(3)` is
`-00:00:00.002`. An **interval** rounds only its `micros` field: months and days are a
*range* spec's business, and a bare `interval(p)` is `INTERVAL_FULL_RANGE`, so
`'1 day 1.5 secs'::interval(0)` is `1 day 00:00:02` on both. A `time` carry is **not**
wrapped — `'23:59:59.9999'::time(3)` is `24:00:00` on both, which `Value::Time`'s raw
`i64` represents.

**`p = 6` is a captured modifier whose rounding is a no-op**, distinct from a bare
`timestamp` that carries none: 18.6 records `atttypmod = 6` for the first and `-1` for
the second, and `datetime_precision` reads `6` for both. A `p` **above** 6 is clamped to
6 rather than refused — 18.6 warns (`TIMESTAMP(9) precision reduced to maximum allowed,
6`) and records `6`; the warning is the one half pglet does not reproduce, its notice
channel hanging off a `QueryResult` the type-name reader cannot reach.

**One number, and it must fit `int32`.** 18.6's grammar spells both windows with a
single `Iconst` (`TIMESTAMP '(' Iconst ')' opt_timezone`, `ConstInterval '(' Iconst
')'`), so `timestamp(3,1)`, `timestamp(3.5)`, `timestamp(-1)` and
`timestamp(99999999999999)` are each `42601 syntax error` there — and each is `42601`
here, through pglet's own sentences. `timestamp(2147483647)` does fit and clamps.

**Where the precision goes in a written-out name is three different places**, and
`docs/deparse.md` § A constant prints in its type's own spelling owns that rule
(`cast_type_spelling`): the
canonical name the parser produces carries it **last** (`time with time zone(3)`, so
`decompose_type_name`'s cut on the first `(` cannot read a tz-time as a `time`), the
dump and `Portable` rendering carry it after a one-word name (`timestamp(3)`), and the
`Catalog` rendering puts it where `format_type` does — **inside** the words
(`timestamp(3) without time zone`).

**Residual: a rounding carry out of range is loud here and a value there** — a member of
§ The date and timestamp domain's pglet-only refusals.
`'9999-12-31 23:59:59.9999'::timestamp(3)` carries into year 10000, which 18.6
represents and `OffsetDateTime` does not, so pglet raises
`22008 timestamp out of range` (`ERRCODE_DATETIME_VALUE_OUT_OF_RANGE`, the code 18.6
itself spends when a timestamp leaves its range) where 18.6 answers
`10000-01-01 00:00:00`. Loud, pglet-stricter, and never a silently-kept unrounded value.

## `timestamptz` text encoding (load-bearing)

`timestamptz` has no distinct value (stored as UTC `timestamp`) — only the
**column** `TypeOid` carries the tz/no-tz distinction, so the wire **text**
encoder is column-type-aware (`value_to_text_with_oid`): a `timestamptz` column
renders with the UTC `+00` offset (`… 12:30:45.123+00`), a plain `timestamp`
without one. The offset matters — a text-protocol driver (node-postgres, the
fuz_app matrix) reparses an offset-less value in its own local zone and
silently shifts the instant; pglet is always UTC, ≡ Postgres at `TimeZone=UTC`.
**The session's zone and styles are fixed, and a statement that would move one is
refused** (`0A000`) rather than accepted and ignored: `TimeZone` is `UTC`, `DateStyle`
`ISO, MDY`, `IntervalStyle` `postgres`. `SET`, `SET TIME ZONE`, `set_config` and the native
server's startup packet reach one decider (`catalog::commands`' `check_fixed_guc`, whose doc
comment lists the spellings it honors), and a reset — `DEFAULT`, `SET TIME ZONE LOCAL`,
`RESET` — restores the value pglet has. 18.6 acts on each of these: after `SET TIME ZONE
'America/New_York'`, `'2024-01-01 03:00+00'::timestamptz::date` is `2023-12-31` there
(measured). Real zones are `EDGES.md` row 69's open residual.
The binary encoder is offset-correct by construction (the OID tells the driver it's
UTC), so the differential fixtures' binary path cannot see this — a `-- @text` fixture
(`types/timestamptz_text.sql`) guards it.

**An array's elements take the same rule, at the declared element type.** A
`timestamptz[]` renders each finite element with `+00` through that one
`value_to_text_with_oid`, which renders an array at its declared type's element rather
than at the element type the `Value::Array` records — `ARRAY[timestamptz '…']` records
`timestamp`. Measured on both engines: the simple-query and extended text formats, `COPY TO`,
the dump, a `::text` / `CAST(… AS varchar)`, and `concat` and `concat_ws` (which read each
argument's declared type, so a scalar `timestamptz` argument keeps its `+00` too); and the
wasm host's JS array carries it. Without it the dump's `COPY` would load each element at
the restoring session's zone: a shifted instant under `psql -f` at `America/New_York`,
measured. Guarded by
`types/timestamptz_array_text_offset.sql` (its text leg), `copy/timestamptz_array_offset_to.sql`
and the `test_dump` round-trip, whose Postgres restore runs at a non-UTC zone.

**The declared element type reaches two more places.** A cast of a `timestamptz[]` to
`text[]` or `varchar[]` renders each element with `+00` (`eval_cast`), as 18.6's
per-element `timestamptz_out` does. And a **binary-format** array's header names the result
column's element type where every non-`NULL` element is sent as that type, and the element
type the value records otherwise (`value_to_binary_with_oid`, `value_sends_as`; `EDGES.md`
row 36). A `timestamptz[]` column's `Value::Timestamp` elements are sent as `timestamptz`, so
an `ARRAY[timestamptz '…']` result's header says 18.6's `1184` (not `1114`), and an
`ARRAY['a'::varchar]` result's says `1043` (not `25`). The element bytes are the same
either way, and the fixture harness's binary leg decodes an array by the column's element type, so the
server unit test `binary_array_header_carries_the_columns_element_type` guards the header
over a raw `DataRow`. Each measured on both engines. A bare `$N` whose type the client declared at
`Parse` is a result column of that type, so a `timestamptz` one renders with `+00` and an
`ARRAY[$1]` over it is headed `1184` (`docs/param_binding.md` § A declared type is part of
parse analysis).

## Fail-loud gaps — `TIMETZ` and any unrecognized type

`TIMETZ` (`time with time zone`) is unimplemented as a **column type** — pglet
is UTC-only; plain `TIME` is real. The `CAST`/`::` path and the DDL column path
share **one** type-name reader (`parser::parse_type_name`) and **one** resolver
(`executor`'s `resolve_type_name`, entered as `resolve_column_type` at both), so a multi-word or
length-bearing spelling (`double precision`, `varchar(255)`) parses on both, and
`timetz` and an unrecognized name each answer the same `42704` at a cast and at a
column — measured, and `42704 type "time with time zone" is not supported` at
`timetz(3)`, `timetz`, `time(3) with time zone` and `time with time zone` alike. **A shared reader is not a shared answer**, though, because the
two positions raise at different points: a column definition leaves an unread `(`
to the caller's `,`-or-`)` while a cast resolves inline, so `json(3)[]` is `42601`
at a column and `42704` at a cast, and `bytea(3)[]` is `42601` at a column and
`22023` at a cast (measured; `EDGES.md` row 120's bullet carries the split). An unsupported
type returns `Error::UnsupportedType` (SQLSTATE `42704`)
rather than silently coercing to TEXT — the "correct-or-loud, never silently-wrong"
rule at a type name. Legacy lenient coercion is opt-in: a per-connection
thread-local toggle (`planner::set_coerce_unknown_types`, default off),
reachable via `Database::set_coerce_unknown_types` (embed) and the server's
`--coerce-unknown-types` flag.
