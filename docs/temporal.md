# Temporal types — DATE, TIME, INTERVAL, timestamps

This doc covers the temporal types in full. The short summary is in the repo `CLAUDE.md`
§ Type system. NUMERIC has its own doc ([numeric.md](numeric.md)), and `types/interval.rs`
documents INTERVAL's internals inline at the same level of detail. pglet is UTC-only
throughout, and that shapes every divergence below.

## The date and timestamp domain

**A finite `date`, `timestamp` or `timestamptz` value is from `0001-01-01` through
`9999-12-31`, reading a `timestamptz` in UTC. A finite value outside that range is refused
and never produced.** `infinity` and `-infinity` aren't finite values and aren't refused
(§ `infinity` and `-infinity`). 18.6's ranges are much wider (a date from 4714-11-24 BC to
5874897-12-31, a timestamp to 294276-12-31), and it prints years before 1 with a ` BC`
suffix. pglet doesn't model BC. Its values are `time`'s `Date` / `OffsetDateTime`, whose
years run from -9999 to 9999. Those types can represent a year 0 or a negative year, and
the text encoder prints it (`0000-12-31`), but no Postgres input function reads that back.
A table holding one dumps to a `COPY` that 18.6 refuses, and `psql -f` exits `0` with the
table empty.

The check is `pglet_core::types::check_date_domain` / `check_timestamp_domain`. The second
judges an instant in UTC and returns it at UTC. These are the places that call it:

| producer or door | where | answer past an edge |
| --- | --- | --- |
| text input: `parse_date`, `parse_timestamp`, `parse_timestamp_naive` (a cast from text, a typed constant, a `COPY FROM` field, a text-format parameter, an array literal's element and a value the wasm host binds each reach one of them) | `types/text.rs` | for a timestamp, each field gets its own verdict at the point 18.6's field loop raises it (§ Reading a timestamp spelling). A numeric UTC offset outside 18.6's ranges is `22009 time zone displacement out of range: "<input>"`, and a time of day out of range is `22008 date/time field value out of range: "<input>"`. After the loop, the date's fields **as written** (a year 0, a month or day out of range) get that same `22008`, before an offset or a rounding moves the instant. An offset, a sub-second rounding, a seconds field of 60 or an hour of 24 that carries the UTC reading across either edge is `22008 timestamp out of range`. A spelling the reader refuses as malformed is `22P02` for a timestamp (`EDGES.md` row 96). A date's year past 9999 is `22008 date out of range` |
| binary input: the `date`, `timestamp` and `timestamptz` receive arms | `types/binary.rs` | `22008 date out of range` / `timestamp out of range`, for every count outside the domain except the two sentinels each type reads as `infinity` / `-infinity` |
| the microsecond rounding of a text fraction, and a seconds field of 60 or an hour of 24 | `decode_timestamp_fields`, then the reader's check after it | `22008 timestamp out of range` when it carries into year 10000 |
| `date ± integer` | `date_shift` | `22008 date out of range` |
| `date ± interval`, `timestamp ± interval`, `timestamptz ± interval` | `timestamp_add_interval` | `22008 timestamp out of range`, judged on the result, with the months and days passes judged against 18.6's range (below) |
| `date + time` | `eval_arithmetic` | `22008 timestamp out of range` |
| `date_trunc` | `date_trunc_timestamp` | `22008 timestamp out of range`. You can reach this from inside the domain: `decade` over years 1–9 gives year 0 |
| a `(p)` sub-second rounding | `round_timestamp_to_precision` | `22008 timestamp out of range` (§ Sub-second precision) |
| `now()`, `CURRENT_TIMESTAMP`, `CURRENT_DATE`, `LOCALTIMESTAMP`, the other clock reads | `builtins::wall_clock` | `22008 timestamp out of range` for a clock before year 1. On wasm, a JS clock past year 9999 gives an instant `OffsetDateTime` can't hold, which is `22000` (§ The transaction and statement clocks) |
| the `date → timestamp` / `timestamptz` and `timestamp → date` casts | `executor::coerce` | `22008 date out of range` / `timestamp out of range`. The `→ date` direction reads the instant's UTC date |
| a `Value` bound through `Database::execute_bound` | `normalize_value_domain`, after `bind` | the element's own refusal. A `timestamp` bound at another offset is converted to UTC, and one carrying nanoseconds below the microsecond is truncated toward the earlier instant (below) |
| a value a write stores, and the key an `ON UPDATE CASCADE` converts for a referencing column, whether or not a child row takes it | `coerce_column_value`, last (`normalize_stored_value_domain`). For the action's key, all of `coerce_column_value` runs, the `(p)` rounding included, from `executor::dml`'s `action_assigned_values`, once per firing and before looking up the rows the action rewrites | the element's own refusal. A `timestamp` is stored at UTC and at a whole microsecond. Only the columns a write assigns get here: an `UPDATE` of another column rewrites the row with its other values as stored. The action's key is refused even with no child row to store it: a `timestamp(0)` or `timestamptz(1)` child of a key moved to a value its rounding carries into year 10000 is `22008 timestamp out of range`, where 18.6's action matches no row and the `UPDATE` goes through (`EDGES.md` row 142) |
| the dump's stored rows (not its stored expression text) | `dump::temporal_restore_failure`, called by `refuse_unreadable_values`'s row walk | the export is refused: `22008 date out of range` / `timestamp out of range` for a value whose UTC reading or rendered fields are outside the domain, and `22000 cannot dump a timestamp stored at UTC offset …` for one at another offset, since its text would reload as a different instant |

**The domain is judged on the UTC reading, but some operations read an instant at its own offset.** Comparison,
ordering and the `→ date` cast read an instant in UTC. The text renderers print its fields
at the offset it carries, and `+ interval`, `extract` and `date_trunc` take their calendar
fields from that offset too. The producers in the table build their instants at UTC, so the
two readings agree there. A caller of `execute_bound` can pass an instant at another
offset, like `0000-12-31 22:00:00-05`, which is `0001-01-01 03:00:00` in UTC. So that entry
point, like the store, converts the value to UTC instead of only checking it.
`GREATEST`/`LEAST`, `MIN`/`MAX` and `COALESCE` pick a value rather than build one, and have
no check.

**A value that arrives already built is cut to whole microseconds at the same two entry
points.** 18.6 holds no precision below the microsecond, and pglet's store persists the
binary format's microsecond count. Without this, a `timestamp` bound with nanoseconds (what
`OffsetDateTime::now_utc()` builds) would compare by them in memory but not after the store
is reopened. Untruncated, over a file-backed store, `ts = timestamp '2026-01-01 00:00:00'`
against a bound `2026-01-01 00:00:00.000000999` returns no rows and then one, and a bound
`1969-12-31 23:59:59.9999995` renders `…23:59:59.999999` and then `1970-01-01 00:00:00`
(we tested both). These entry points **truncate toward the earlier instant**, which is
`now()`'s rule and not text input's. 18.6's `'1969-12-31 23:59:59.9999995'::timestamp`
rounds to `1970-01-01 00:00:00` (tested), while pglet's embed entry point gives
`1969-12-31 23:59:59.999999`. That's our choice, not something we measured from 18.6, which
has no way to take an already-built instant with nanoseconds (its binary format carries
none). The reasons: the value an embedding caller is most likely to bind with nanoseconds
is a clock read, and truncating it gives the instant `now()` would. And the text renderer
already prints an instant's microsecond field that way, so no value renders differently
because of it. `crates/pglet/tests/temporal_infinity.rs` checks it across a reopen.

**Where 18.6 also refuses, pglet gives 18.6's code and message, in the cases we tested**:
an offset 18.6 refuses next to a year-0 field or that carries the instant past an edge
(`'0000-12-31 23:00:00-16'::timestamptz`, `'0001-01-01T00:00:00+23:59'::timestamptz`,
both `22009`), a signed year whose digits 18.6 reads as an out-of-range offset
(`'+9999-12-31 23:00:00-05'::timestamptz`, `22009`), a year-0 field,
`'0001-01-01'::date - 2147483647`, `'9999-12-31'::date + 2147483647`,
`timestamp '0001-01-01' - interval '10000 years'` and its `date` twin, and
`timestamp '2000-01-01' - interval '-2147483648 days'` (`22008 interval out of range`).
It does **not** match 18.6 in these cases, each tested:

| statement | 18.6 | pglet |
| --- | --- | --- |
| `'+0000-12-31 12:00:00'::timestamp` | `22007 invalid input syntax` | `22P02` |
| `timestamp '2000-01-01' - interval '-2147483647 mons -2147483647 days -2562047788:00:54.775807'`, and its `date` and `timestamptz` twins | `22008 interval out of range` | `22008 timestamp out of range` |

The first reads its `+0000-12-31` as an offset whose hour is in range and whose field is
malformed, which gives row 96's code. The second negates to the field triple 18.6 reads as
`infinity`. 18.6's negation refuses that, and pglet's doesn't, because pglet has no
infinite interval (`EDGES.md` row 143).

**Where 18.6 returns a value (a `BC` result, or a year past 9999), the refusal is pglet's
own.** It's `22008`, with the bare `date out of range` / `timestamp out of range` message
18.6 itself uses when a value leaves its range, except for a timestamp literal the grammar
refuses first (`22P02`, the table's first row). So `'9999-12-31'::date + 1` is `22008`, the
code `date_pli` itself uses where 18.6's range ends, and not the `22003` of an integer
overflow. These refusals are `EDGES.md` row 142, along with what they cost a binary
parameter.

**`± interval` checks its months and days passes against 18.6's range, and the result
against the domain.** `timestamp_add_interval` runs the months, days and microseconds passes
over integers, and refuses a months or days pass outside Julian day 0 (`4714-11-24 BC`)
through `294276-12-31`. The microseconds pass has no range check of its own. Its `i64`
overflow check and the domain check on the result do that job. So
`date '9999-12-31' + interval '1 day -1 microsecond'` is `9999-12-31 23:59:59.999999`
on both engines.

**The store's finalizer walks an array only when its declared element type is temporal**
(`normalize_stored_value_domain`). That gate relies on where the finalizer's arrays come
from. This is reasoned from the producers, not enforced by the type and not measured: the
readers and the array constructor build an array's elements at its declared element type,
and `Database::execute_bound`, where a caller can pass an array whose declared element type
doesn't match its elements, walks every element before any write sees it. A store written
without the domain check could hold such an array, and the gate would let a write copy it.
But such a store is older than `PGMO`, which reopen refuses, and the dump still refuses the
copy.

**A store written without the domain check isn't opened, and its rows would be refused
anyway.** Every such store has a store magic older than `PGMO`, which reopen refuses
([storage.md](storage.md) § Store versioning). The guards below cover a value put into
storage directly, which is how `crates/pglet/tests/temporal_domain.rs` tests them. Here's
what we found on a store holding a year-0 `date`, `timestamp` and `timestamptz`. Each
renders as stored, and the `date` casts to `text` and compares. A cast that builds a new
value from one (`d::timestamp`, `t::date`, `tz::date`) is refused, and so is a write that
would store one (`INSERT … SELECT` of the value, and an `UPDATE` assigning it back). An
`UPDATE` that doesn't assign the column rewrites the row with the value as stored. The dump
refuses such a row with the value's `22008` instead of writing the `COPY` line 18.6 would
refuse. Deleting the row lets it export. Assigning one such column an in-domain value
leaves the others to refuse (tested: with the `date` assigned, the export was then refused
at the `timestamp`). No migration rewrites such a store.

**The dump refuses on rows, not on the catalog.** It reads a stored expression's text as it
was written and doesn't check it. This build refuses an out-of-domain constant in a
`DEFAULT` (`22008`, from the plan-time constant check), but accepts one in a `CHECK` or a
partial-index predicate over an **empty** table. 18.6 refuses both with `22008`. That's the
stored-expression constant-cast member of `EDGES.md` row 51. We measured what each shape
loses on stores an older build wrote (older than `PGMO`, so this build won't open them),
each shape in its own store. `DEFAULT '0000-12-31'` on a `date` or `timestamp` column,
`CHECK (d <> date '0000-12-31')`, or a partial index `WHERE d > date '0000-12-31'` exports
with exit `0`, and loading that dump into 18.6 with `psql -f` also exits `0`, with things
missing. The `DEFAULT`'s `CREATE TABLE` is refused, so that table and its rows are lost.
Its `COPY` block, sent to a table that doesn't exist, leaves psql reading the rows as SQL,
which loses the rows of the tables dumped after it too. The `CHECK` and the partial index
are each refused on their own, and their tables and rows load without them.

## `infinity` and `-infinity`

**Each of the three types holds `infinity` and `-infinity` alongside its calendar values, as
separate variants. They're never a calendar value that happens to render as the word.**
`Value::Date` and `Value::Timestamp` carry `pglet_core::types::Infinite` (`NegInfinity`,
`Finite`, `Infinity`), so code that reads calendar fields has to match the `Finite` arm by
name. The wire, the store and the index key all carry 18.6's binary count: days or
microseconds since `2000-01-01`, with `i32::MIN` / `i32::MAX` (`DATE_NOBEGIN` /
`DATE_NOEND`) and `i64::MIN` / `i64::MAX` (`TIMESTAMP_NOBEGIN` / `TIMESTAMP_NOEND`) for the
infinities. So the key order is the value order. The store magic moved to `PGMO` with that
encoding ([storage.md](storage.md) § Store versioning).

**Spelling.** The text readers accept the word in any case, with an optional `+` or `-`,
and whitespace (C's `isspace`) around the word and between the sign and the word
(`' - Infinity '`). They render `infinity` / `-infinity`, and a `timestamptz` with no
`+00`. That's narrower than 18.6's tokenizer. The difference comes from how the tokenizer
works, so it isn't a fixed list of spellings: `ParseDateTime` skips an ASCII punctuation
byte (`ispunct`) as a field delimiter wherever it doesn't give that byte a role of its own,
and it reads the word as one field among others. So the word next to such a byte, or next
to an `AD` / `BC` field, is still `infinity` there. Each of these is accepted by 18.6 and
refused here (`22007` for a `date`, `22P02` for a timestamp) at all three types, tested:
`'infinity;'`, `'(infinity)'`, `'infinity:'`, `',infinity'`, `'infinity,'`, `'!infinity'`,
`'infinity@'`, `'infinity_'`, `'{infinity}'`, `'"infinity"'`, `'infinity ""'`,
`'infinity BC'` and `'-infinity ad'`. The bytes the tokenizer does give a role (`.`, `/`,
`-` and `+` directly after the word) are refused by both (`'infinity.'`, `'infinity/'`,
`'infinity-'`, `'infinity+'`, tested, with row 96's `22P02` against 18.6's `22007` for a
timestamp), and so are `'infinity 1'`, `'infinity today'` and `'infinity utc'`.

**What each operation returns.** Every row was tested on both engines at the operand types
it names:

| operation | answer, both engines |
| --- | --- |
| order, `=`, `min`/`max`, `greatest`/`least`, `GROUP BY`, `DISTINCT`, an index | `-infinity` before every value, `infinity` after; a `date` infinity equals the timestamp infinity of the same sign |
| `date ± integer`, `date + time`, `date_trunc`, a `(p)` modifier, the `date`/`timestamp`/`timestamptz` casts | the same infinity, after the operation's other checks |
| `date`, `timestamp` or `timestamptz` `± interval`, the interval not one of the field triples 18.6 reads as an infinite interval (below) | the same infinity, after the operation's other checks. `timestamp 'infinity' - interval '-2147483648 days'` is still `22008 interval out of range` |
| `timestamp` or `timestamptz` `AT TIME ZONE 'UTC'` | the same infinity |
| `timestamp::time`, `timestamptz::time` | NULL |
| `to_char` of a `timestamp` or `timestamptz` | NULL, whatever the format |
| `isfinite(date \| timestamp \| timestamptz)` | `false` |
| `date_part` of `year`, `decade`, `century`, `millennium`, `julian`, `isoyear`, `epoch` | `Infinity` / `-Infinity`; every other recognized field NULL |
| `EXTRACT` of a cycling or zone field | NULL, except `EXTRACT(… FROM date)` of a time-of-day or zone field, which is `0A000` as for a finite date |
| `date - date` with an infinite operand | `22008 cannot subtract infinite dates` |
| an infinity less the same infinity | `22008 interval out of range` |

**Where the two differ.** pglet refuses two of 18.6's answers loudly instead, each with
`0A000`, because the type 18.6 answers in has no infinity here. The first is a timestamp
difference with one infinite operand, or two of opposite signs. 18.6 returns an infinite
`interval`, and pglet says `not implemented: infinite interval` (`PgInterval` has no
infinity, `EDGES.md` row 143). The second is `EXTRACT` of a field that `date_part` returns
`±Infinity` for. 18.6 returns a `numeric` `Infinity`, and pglet says
`not implemented: numeric Infinity` (`PgNumeric` has none). A third doesn't need a missing
type and is refused anyway: `date_part`'s `float8` `Infinity` cast to `numeric`
(`date_part('year', date 'infinity')::numeric`) is `22003 numeric out of range` here and
`Infinity` in 18.6, because the `float8 → numeric` cast refuses a non-finite value,
whatever produced it. These also differ, each tested:

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
| `isfinite(interval '1 day')` | `t` | `42883`: the `interval` overload isn't modelled |
| `isfinite(time '01:00')` | `t`, through `time`'s implicit cast to `interval` | `42883` |
| `sum(tm)`, `avg(tm)` over a `time` column | the sum or average as an `interval`, through the same cast; no row where no group forms | `42883`, including a statement that forms no group (`EDGES.md` row 307) |
| `isfinite(NULL)`, `isfinite(NULL::text)`, and `isfinite(i)` over an `int` column holding one NULL, or over an empty table | `42725` for the first, `42883` for the others | NULL, or no rows (`EDGES.md` row 144) |

`<max>` is `interval '2147483647 mons 2147483647 days 2562047788:00:54.775807'` and `<min>`
is `interval '-2147483648 mons -2147483648 days -9223372036854775808 microseconds'`. Those
are the field triples 18.6 reads as `infinity` / `-infinity` and pglet reads as finite
intervals (`EDGES.md` row 143). Added to `infinity` or subtracted from `-infinity`, `<max>`
leaves the infinity unchanged on both engines, and so does `<min>` added to `-infinity`.
Each was tested at all three types.

## DATE

A full column type: `Value::Date`, `YYYY-MM-DD` text, i32 days from 2000-01-01 in binary,
B-tree indexable, comparable, with casts text↔date and `timestamp::date`. Arithmetic is
complete: `date ± int → date`, `date - date → int4`, `date ± interval → timestamp` (in
`eval_arithmetic`, with the result OID inferred in `temporal_arith_type`). Text input
accepts a 1- or 2-digit month and day (`2026-1-1`, like Postgres). A malformed spelling is
`22007` (`invalid_datetime_format`), and a well-formed but out-of-range field
(`2026-13-01`, `2026-02-30`) is `22008` (`datetime_field_overflow`). A year with a leading
`-` is refused loudly, since pglet doesn't model BC. It's `22007`, which is also PG's code
for `'-0001-01-01'`, but it's a **divergence** for `'-2026-06-15'`, where PG reads the sign
as a time zone displacement and answers `22009`.

`CURRENT_DATE`/`CURRENT_TIMESTAMP` are Postgres **SQL value functions**
(`SqlValueFunction`): parenless special forms the parser recognizes, not functions. So
`current_timestamp()` fails loudly (`42601`) like Postgres, in every position, including a
column `DEFAULT`. Postgres does accept the precision form `CURRENT_TIMESTAMP(p)`, which
pglet rejects along with any other parens there. That's the *value function's* grammar,
not the type's; the type does model a sub-second precision (§ Sub-second precision).
`CURRENT_TIME`/`LOCALTIME` are wired and return plain `time` (see § TIME). `LOCALTIMESTAMP`
is wired too: it's the `now()` instant typed as `timestamp` without a zone, so its text has
no `+00` offset. The non-temporal value functions (`CURRENT_USER`, `SESSION_USER`, `USER`,
`CURRENT_ROLE`, `CURRENT_CATALOG`, `CURRENT_SCHEMA`) are special forms too
([compatibility.md](compatibility.md)'s *Session information functions* row). That's why
`SqlValueFunction` has a `takes_no_argument_list` split: `current_schema` is the one member
18.6 also has as a `pg_proc` entry, so a `(` after it is a call, not the `42601` every other
member gives. `DEFAULT CURRENT_DATE` is wired, and `DEFAULT LOCALTIMESTAMP`/`LOCALTIME` are
accepted through the `ColumnDefault::Expr` catch-all rather than a dedicated arm.

### Reading a timestamp spelling

**A timestamp spelling is split into fields the way 18.6's `ParseDateTime` splits it, and
the fields are read the way its `DecodeDateTime` reads them.** That's
`split_datetime_fields` and `decode_timestamp_fields` in `types/text.rs`, for the field
kinds pglet models. Whitespace separates fields, and so does any other ASCII punctuation
byte the tokenizer gives no role. So `2000-01-01 12:34:56`, `2000-01-01/12:34:56`,
`2000-01-01_12:34:56`, `2000-01-01:12:34:56` and `2000-01-01#12:34:56` are the same value
on both engines. The bytes that do have a role are the date's `-`, `/` and `.` delimiters,
a time's `:` and `.`, and a sign. `2000-01-01-12:34:56` and `2000-01-01.12:34:56` run the
time into the date field and are malformed. In `2000-01-01+12:34:56` the `+` starts an
offset, so the value is midnight at `+12:34:56` (`1999-12-31 11:25:04+00`). A letter
starts a word, and `t` in either case between the date and the time is 18.6's ISO filler.
A word that runs straight into a `-`, `/` or `.` continues as one field, read as a zone
name (below). So does a word running straight into a digit or a `+`, unless the word is
one of 18.6's own date tokens (`datetktbl`: `t`, a month or day name, `am`, `ad`, …), which
stays a word. So `abc5`, `abc+5`, `july-5` and `utc+16` after a date are zone names on both
engines, while `t5`, `t+05` and `july5` are refused on both. A control byte or a non-ASCII
byte is malformed. We tested all of these.

The fields pglet models can each appear at most once and **in any order**, except that a
date after a time of day is malformed (`00:00:00 2000-01-01`, as `DecodeDate` refuses it).
They are: a `y-m-d` date whose year has at least three digits (`200-01-01` is year 200;
`2000/01/01`, `2000.01.01`, `2000-01-001` and a trailing `2000-01-01-` read the same), a
time of day, a numeric offset, the word `z`, a POSIX zone after the date, and the `t`
filler after the date.

**A time of day is read the way `DecodeTime` reads it.** That's `hh:mm`, `hh:mm:ss`, and,
when a fraction follows two fields, `mm:ss.f` (`12:30.5` is `00:12:30.5`). An empty field
is `0` (`12::30` is `12:00:30`, `12:` is noon). Each field is range-checked on its own (a
minute up to `59`, a **second up to `60`**), and then `time_overflows` limits the whole
time of day to at most `24:00:00`. So a seconds field of 60 is accepted **at any time of
day**, not only at a month's end, and carries into the next minute (`2000-06-29 23:59:60`
is `2000-06-30 00:00:00`). A fraction on top of it is accepted when the total stays within
the day (`12:30:60.5` is `12:31:00.5`) and refused when it doesn't
(`'2000-06-30 23:59:60.5'` and `'2000-01-01 24:00:00.5'` are
`22008 date/time field value out of range: "<input>"` on both). An hour of 24 is exactly
`24:00:00`, the next day's midnight. A carry out of the date domain gets the domain's
refusal: `'9999-12-31 23:59:60'` is `10000-01-01 00:00:00` on 18.6 and
`22008 timestamp out of range` here (`EDGES.md` row 142).

**A fraction is `rint(strtod(fraction) * 1000000)` over a C `double`**, so a tie goes to
the even microsecond. But it's the `double` that ties, not the decimal: on 18.6 `.0001255`
is `.000125` and `.0001265` is `.000127` (neither is a tie once it's a `double`), while
`.0000015` and `.0000025` both give `.000002`. `parse_frac_micros` implements that rule for
the timestamp readers and for `time` alike. Tested at both types.

**The error codes follow 18.6's field loop.** A time of day out of range is reported where
18.6's loop reaches it, so it wins over an out-of-range offset written after it
(`'2000-01-01 25:00:00+16'` is `22008` on both) and loses to one written before it. The
date's own fields (a year 0, a month outside `1 ..= 12`, a day outside `1 ..= 31` or past
its month's end) are checked after the loop, `22008 date/time field value out of range` on
both. 18.6 adds a `DateStyle` hint to the month and day-range message, which pglet doesn't
send. A spelling the reader refuses as malformed is `22P02 invalid input syntax for type
timestamp` here, against 18.6's `22007`. That's `EDGES.md` row 96, tested on its five
routes.

**A numeric UTC offset is read the way 18.6's `DecodeTimezone` reads it.** A sign, then
any whitespace after it, then a digit and every following digit, `:`, `.` and `-`. That's
an hour, then an optional `:minute` and `:second`, or, with no `:` and more than two
digits, a run-together `hhmm` (`+0530`; `+155` is `01:55`). Each field is read by
`strtoint`, so an empty one is `0`. The hour must be `0 ..= 15` and the minute and second
`0 ..= 59`. Outside those, or past `int`, the literal is
`22009 time zone displacement out of range: "<input>"`. That holds on `timestamp` too,
which only drops the offset after reading it, and at a column `DEFAULT`, which is refused
at `CREATE TABLE`. The range checks run before the check that the field was read whole
(`+16.5` is `22009`, while `+05.5` is malformed: row 96's `22P02` here, `22007` in 18.6).
An offset is its own field wherever it appears: after an empty fraction
(`'2000-01-01 00:00:00.+16'`), as a second offset (`'…+05+16'`, whose range is checked
before it's found to be a second one), before the date (`'+05 2000-01-01 00:00:00'`), and
in a signed year, whose digits are an offset (`'+2026-06-15 12:00:00'` is `22009` on both,
`'+0000-12-31 12:00:00'` malformed). We tested all of these.

**A zone name is only read as a POSIX zone with no daylight-saving part**, through
`posix_zone_seconds`: a name, then `[+|-]hh[:mm[:ss]]` with the hour at most `167`, the
minute `59` and the second `60`, ending the field. The offset counts **west**
(`'2000-01-01 00:00:00 utc+16'::timestamptz` is `2000-01-01 16:00:00+00`, and
`2000-01-01T00:00:00Z+05` reads as `z+05`). The word `z` is UTC. Anything else a zone name
can be is refused as malformed here, where 18.6 reads it: a named zone or abbreviation
(`EST`, `UTC`, `America/New_York`), and a POSIX zone with a daylight-saving part, which
18.6 resolves against default rules, so `'2000-06-30 00:00:00 x5z'::timestamptz` is
`04:00:00+00` in June there. A letter written where a separator would go is read the same
way. `'2000-01-01x12:34:56'` is the POSIX zone `x12:34:56` on both engines: `12:34:56+00`
as a `timestamptz`, and the date's midnight as a `timestamp`, which drops the zone.
`'2000-01-01x12:34:56.5'` is a value in 18.6, read as a zone with a daylight-saving part
(`x12:34:56`, then `.5`), and malformed here. `'2000-01-01x12:34:56+05'` is
`22023 time zone "x12:34:56+05" not recognized` in 18.6 and malformed here. Each refusal
here is `22P02`, so none of these is silent.

**Other spellings 18.6 reads and pglet refuses as malformed**, each tested as a
`timestamp`: a month name (`Jan 1 2000`), a year and a day of year (`2000-060` and
`2000.060` are `2000-02-29 00:00:00` in 18.6, while `2000-060-01` is `22007` on 18.6 too),
a year of one or two digits (read in `DateStyle` order in 18.6: `01-02-03` is
`2003-01-02`), a run-together `hhmmss` (`2000-01-01 123456`), `am`/`pm`
(`2000-01-01 12:00 pm`), the `at`/`on` noise words (`2000-01-01 at 12:00`), the special
words `epoch`, `now`, `today`, `tomorrow` and `yesterday`, and `allballs` after a date
(`2000-01-01 allballs` is midnight in 18.6; a bare `allballs` is `22007` on 18.6 as a
`timestamp` or a `timestamptz`). The `date` and `time` readers are separate and narrower
still. They refuse an offset after a time of day with `22007` (`'00:00:00+05'::time`,
`'2000-01-01 00:00:00+05'::date`), where 18.6 reads the offset and drops it. And the
`date` reader refuses a trailing time of day (`'2024-01-01 00:00'::date` is `22007` here
and `2024-01-01` in 18.6).

## The transaction and statement clocks

**The clock functions read one of three clocks, and the spelling decides which.** This is
18.6's split (`xact.c`'s `xactStartTimestamp` and `stmtStartTimestamp`, and
`GetCurrentTimestamp`), checked against the oracle for each spelling:

| clock | spellings | the instant |
| --- | --- | --- |
| the transaction's start | `now()`, `transaction_timestamp()`, `CURRENT_TIMESTAMP`, `LOCALTIMESTAMP`, `CURRENT_DATE`, `CURRENT_TIME`, `LOCALTIME` | the start of the statement that began the transaction: an open block's `BEGIN`, or with no block open, the statement itself |
| the statement's start | `statement_timestamp()` | when the client's message arrived |
| the wall clock | `clock_timestamp()` | the call |

Two reads of a transaction-start spelling inside one transaction give the same value at
every place we tested against 18.6: the select list, a predicate over many rows, a
`DEFAULT` on several columns and many rows, `INSERT … SELECT`, `UPDATE … SET`,
`ON CONFLICT DO UPDATE`, `RETURNING`, a `CHECK`, a CTE and a subquery, the statements of a
`DO` body, an `ADD COLUMN` back-fill, a `COPY`'s defaults, a statement with bound
parameters, and an index probe's constant next to the row filter the stripped build runs
instead (the `clock_*` fixtures). A savepoint and a `ROLLBACK TO` are inside the
transaction and don't move its start. The transaction after a `COMMIT` or a `ROLLBACK`
(an aborted block's included) has its own start.

**Where the instants live** (`pglet_sql::builtins`' module doc has the full statement):

- A host marks each statement's arrival with `builtins::start_statement`. The embed facade
  does it in the entry guard every public entry point goes through (so the wasm bindings,
  which run on the facade, get it there). The native server does it in
  `execute_query_core` and, for a `COPY`, in `dispatch_copy`. The native hosts read the
  clock at that call. On wasm the read is a call out to the JS host, so it happens where
  the statement first needs the instant, and a statement that reads no clock doesn't make
  it at all.
- An open block keeps its start in the connection's `SessionSettings` (`begin_block` takes
  the statement start its `BEGIN` ran under, and `end_block` drops it with the block). So
  it's per connection, and it's set and dropped in the same place as the block's isolation
  level. `SessionSettings::install_ambient`, which each statement already runs, passes it
  to the evaluator along with the session.
- Nothing is kept in the plan: a prepared statement executed twice reads two statement
  starts.

**One message is one statement start.** A simple-query message carrying several statements,
and one `Database::execute` call, runs them all under one instant, and a transaction one of
them begins starts at that instant. `BEGIN; SELECT now() = statement_timestamp()` sent as
one message is `t` on both engines, and `f` sent as two.

**The remaining protocol difference is in the extended protocol** (`EDGES.md` row 292).
18.6 starts the implicit transaction at the first message after a `Sync` and restamps the
statement at each `Bind` and `Execute`. So outside a block, a bound statement's `now()` is
earlier than its `statement_timestamp()`, and the statements of one pipeline share a
`now()`. pglet takes both instants at the `Execute` (the embed and wasm hosts at the call),
so the two are equal there, and each statement in a pipeline has its own.
`now() <= statement_timestamp()` holds on both as long as the clock isn't set back inside
an open block (below).

**The wasm clock is `Date.now()`'s millisecond, stepped within its first half.** A read
returns an instant between the start of the millisecond `Date.now()` reports at that read
and 499 microseconds past it. So a JS clock that moves, forward or back, is followed as it
reads: a test runner's fake timer pinned to a date reads as that date to the millisecond,
however many reads there are. The first read in a millisecond returns its start.

`Date.now()` is whole milliseconds, so a read that finds the previous read in the same
millisecond is placed a microsecond after it (`builtins::step_host_clock`). Through the
first five hundred reads of a millisecond, two statements in it get ordered, distinct
starts, and a `clock_timestamp()` is later than its statement's start (past that, see the
residual below). The last read is kept by the wasm module, not by a database, so the order
also holds between the databases of one module, and they share the steps of a millisecond.

**The tie point is 499 microseconds because a column rounds.** `timestamptz(3)` rounds half
away from the value's zero, 2000-01-01 (§ Sub-second precision): `…00.0005` is `…00.001`
after it and `…00` before it, and `…00.000499` is `…00` on both sides and before 1970. So
every instant a read returns, stored at precision 3, 4, 5 or 6 or unmodified, is inside the
millisecond the JS clock reported, and at precision 3 it is that millisecond. Precision 0,
1 and 2 round the pinned millisecond itself, as they round any instant. One case shows the
step there: a pinned millisecond before 2000-01-01 that's exactly a half at that
precision. Its first read rounds down along with the millisecond, and its later reads round
up.

18.6 reads `gettimeofday` and the native hosts read the OS clock, keeping nothing between
reads, so both also follow a clock that's set back. A set-back doesn't move an instant
already taken: an open block's start stays where its `BEGIN` read it. So inside that
block, `statement_timestamp()` and `clock_timestamp()` are then earlier than `now()`, and
an instant returned before the set-back can be returned again after it.

**The step's residual is the tie.** The five-hundredth read of one millisecond returns the
tie point, and every read after it in that millisecond returns it again. Those reads are
equal (two statements' starts, or a `clock_timestamp()` and its statement's start), and
none is earlier than the one before. 18.6 ties the same way when two reads fall in one
microsecond of its clock. The engine's own readers of the clock (the builtins, the column
defaults, a block's start) take the value and compare nothing against it. A unique key over
the value does compare, and refuses a tied row (`23505`).

Under a frozen clock, that's every read past the five-hundredth, in whichever database of
the module makes it, until a statement reads the clock in another millisecond. Putting the
clock back to the real one restarts nothing until that happens, so tests that pin one
instant share one five hundred between them.

On the real clock, a statement that reads the clock makes one read, plus one more each time
`clock_timestamp()` is evaluated, and a read only ties past the five-hundredth of its
millisecond. So whether a shape ties depends on how many reads the host makes per
millisecond, and a shape near five hundred ties sooner on a faster or idler host. As an
example, here's one idle host we measured, through the npm package under Node. Consecutive
statements made under 100 reads a millisecond, and 100 000 of them left no start tied with
or earlier than the one before. A `clock_timestamp()` evaluated per row made roughly 850 to
1100, so a statement of more than five hundred such rows can repeat a value. An
`ADD COLUMN … DEFAULT clock_timestamp()` back-fill of 100 000 rows left about half its
values distinct (0.47 to 0.57 of them, run to run), and
`INSERT INTO k (ts, v) SELECT clock_timestamp(), x` into a primary key on `ts` was accepted
in every run at 300 rows, `23505` in some runs at 600, and in every run from 1000. 18.6 on
the same host repeated more: about a quarter of that back-fill's values were distinct, and
that `INSERT` was `23505` in every run at every size measured, from 10 rows.

And a step is past the millisecond's start: under a frozen clock the second read is already
later than the JS time bound as a parameter, just as an 18.6 timestamp's microseconds are
later than a JS `Date` taken in the same millisecond.

Measured through the wasm binding and the npm package, with `Date.now` replaced by a
function the test moves, and `P` a pinned millisecond:

| the JS clock | a read answers |
| --- | --- |
| a read under today's clock, then pinned to `2024-01-01T00:00:00Z` | `2024-01-01 00:00:00+00`, in a `DEFAULT now()`, in `CURRENT_DATE`, and in a database made after the pin; the reads after it in that millisecond `…00.000001+00`, `…00.000002+00`, one per read |
| pinned to `2099-01-01T00:00:00Z`, then the real `Date.now` restored | `2099-01-01 00:00:00+00`, then the real clock |
| frozen at `P` across 2500 statements alternating between two databases | the first five hundred starts distinct, in statement order, `P` to `P + 499 µs`; each start after them `P + 499 µs`; none earlier than the one before |
| the same through the npm package, 2400 inserts each followed by a `SELECT now()`, over two instances and a fork | every `now()` read back as a `Date` is `P`; every row's `created_at` is in `[P, P + 1 ms)` |
| frozen at `P`, 1200 rows stamped into `timestamptz(3)`, `timestamp(3)`, `timestamptz(4)`, `timestamptz(5)` and `timestamptz` columns, `P` in 2024, in 1985 and in 1960 | every row of the two precision-3 columns is `P`; every row of the others is in `P`'s millisecond; `now()::timestamptz(3)` past the tie point is `P` |
| frozen at `P`, 1500 rows stamped into `timestamptz(2)`, `(1)` and `(0)` columns | every row is `P` rounded to that precision, except `P` in 1985 exactly on a half (`….005` at precision 2, `….500` at precision 0): the first row rounds down, as `P` does, and the others up |
| frozen at `P`, 1200 rows stamped, `created_at <= $1` with the JS time bound | `1` row, the first |
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

A full column type. `Value::Time` is microseconds since midnight, stored as a raw `i64` so
the `24:00:00` endpoint is representable (unlike DATE's `time::Date`). Text is
`HH:MM:SS[.ffffff]` (trailing zeros trimmed), binary is `int64` (OID 1083, array 1183), and
the index key is plain big-endian (non-negative, so no sign flip; chronological, UNIQUE
enforced). It has full comparison, `ORDER BY` and range. Input accepts `HH:MM:SS[.f]`,
`HH:MM`, `HHMMSS`/`HHMM` (with an optional fractional suffix), and 12-hour `AM/PM`. Casts
are text↔time and `timestamp::time` (truncates to the time of day). Arithmetic:
`time - time → interval`, `time ± interval → time` (uses only the interval's µs and **wraps
mod 24 h**, ignoring days and months, matching `time_pl_interval`), and
`date + time → timestamp`. `CURRENT_TIME`/`LOCALTIME` (value functions) and
`DEFAULT CURRENT_TIME` return plain `time` (PG's `CURRENT_TIME` is `timetz`, but pglet is
UTC-only). Differential-tested (`tests/fixtures/types/time_*.sql`). A malformed spelling
fails loudly with `22007` (`invalid_datetime_format`), and a well-formed but out-of-range
field (`25:00:00`, `12:99`) is `22008` (`datetime_field_overflow`).
**Divergences:** a tz suffix on a time literal fails loudly (`22007`; PG accepts and drops
it), `TIMETZ` (`time with time zone`) stays unimplemented (`42704`), and `CURRENT_TIME(p)`
parens are rejected (`42601`). The `time` reader isn't the timestamp reader's `DecodeTime`
port (§ Reading a timestamp spelling), and it differs from 18.6 at these spellings, each
tested as a `'…'::time` cast:

- **A seconds field of 60** is `22008 time out of range` here. On 18.6 `12:30:60` is
  `12:31:00`, `23:59:60` is `24:00:00`, and `12:30:60.5` is `12:31:00.5`. `23:59:60.5`,
  whose total passes `24:00:00`, is `22008` on both, with different messages (18.6 says
  `date/time field value out of range: "23:59:60.5"`).
- **The `mm:ss.f` reading and an empty field** are `22007` here. On 18.6 `12:30.5` is
  `00:12:30.5`, `12::30` is `12:00:30`, and `12:` is `12:00:00`.
- **`allballs`** is `00:00:00` on 18.6 and `22007` here.
- **A trailing `.` after a run-together `hhmmss` is accepted here**: `123456.` is
  `12:34:56`, where 18.6 refuses it with `22007`. That one is a **silent over-accept**; the
  others above are loud. `123456` and `1234.5` agree (`12:34:56`, `12:34:00.5`).

## INTERVAL

A full column type over an in-house 3-field `PgInterval { months: i32, days: i32, micros:
i64 }` (`pglet_core`'s `types/interval.rs`, no deps). That's Postgres's own layout, and the
fields stay split because a month's day count varies. There are three input grammars:
verbose (`1 year 2 mons`/`ago`), SQL-standard (`1-2`/`3 4:05:06`), and ISO-8601
(`P1Y2M3DT4H5M6S`, where an `M` before `T` is months and after `T` is minutes), with
faithful fractional-unit normalization. Output is `postgres`-style (signed pluralization
`value != 1`, `mon`/`mons`, a forced `+` on a positive group after a negative one, zero →
`00:00:00`). There's a 16-byte binary wire codec (**time → day → month** order, OID 1186,
array 1187) and an order-preserving index key. **Comparison uses Postgres's estimate**,
`micros + (months*30 + days)*USECS_PER_DAY` in i128. So
`INTERVAL '30 days' = INTERVAL '1 mon'` (a documented quirk), a UNIQUE interval index
dedups them, and `ORDER BY`/`GROUP BY`/`DISTINCT` treat them the same (custom `PartialEq`,
like `PgNumeric`'s `1.5 == 1.50`). Arithmetic: `interval ± interval`, unary `−`,
`interval ×/÷ number` (fractions cascade down, byte-for-byte with
`interval_mul`/`interval_div`), `timestamp ± interval` (a 3-pass calendar shift: months
with a leap-aware day-of-month clamp, then whole days, then µs),
`date + interval → timestamp`, and `timestamp - timestamp → interval` (justified via `justify_hours`). There's
no `infinity`: the field triples 18.6 reads as `infinity` / `-infinity` are finite
intervals here at the text inputs and arithmetic we tested, and a binary parameter holding
one is refused (`EDGES.md` row 143). Numeric fields parse exactly (i64, not f64; exponent
forms rejected). It isn't DST-aware, which is only correct because pglet is UTC-only (a
day is always 24 h).

**Fail-loud gaps:** an `INTERVAL <fields>` **range** spec (`DAY TO SECOND`, `SECOND(0)`,
…). The fields restriction changes semantics, so pglet rejects it rather than truncate
wrongly. At a **column**, every spelling we tested is refused: `interval day to second`,
`interval second(0)`, and the single-field `interval year` and `interval second` are each
`42601 syntax error … expected ')'` here, against an accepted `CREATE TABLE` in 18.6. At a
**cast**, it's only refused when tokens are left unread. `'1 day 2 hours'::interval day to
second` and `'1.5 secs'::interval second(0)` are `42601` here against `1 day 02:00:00` and
`00:00:02` in 18.6. But a **single-field** spec is swallowed as a column **alias**:
`'1 year 2 mons'::interval year` returns `1 year 2 mons` here against 18.6's `1 year`,
which is a *silent* wrong value. That's the class `EDGES.md` row 71 records; its two
`ISNULL` / `NOTNULL` spellings are closed, and this is its live member. Each was tested on
both engines. A bare `interval(p)` is a *precision*, not a range, and is applied
(§ Sub-second precision). A malformed spelling is `22007` (`invalid_datetime_format`), a
**parse-time field overflow** (`interval '999999999999 years'`) is `22015`
(`interval_field_overflow`), and an **arithmetic** overflow (`interval × n`,
`timestamp ± interval`) is `22008` (`datetime_field_overflow`).

## Sub-second precision — the `(p)` four of these types carry

**A modifier the type carries is kept, clamped to the type's own maximum, and enforced. It's
never discarded.** `timestamp(p)`, `timestamptz(p)`, `time(p)` and `interval(p)` are the
four (`pg_type.typmodin` is non-zero for each; `timetz` would be a fifth, and it's `42704`
here whatever the modifier). This is the third part of [coercion.md](coercion.md) § A
canonical type name decomposes once.

**The rounding rule matches 18.6, and the sign it uses isn't the wall clock's.**
`AdjustTimestampForTypmod` (`timestamp.c`), `AdjustTimeForTypmod` (`date.c`) and
`AdjustIntervalForTypmod`'s sub-second half are three copies of the same arithmetic: add
half the rounding unit, then truncate toward zero. The sign each one branches on is the
value's **internal** representation: microseconds since **2000-01-01** for a timestamp,
since midnight for a `time`, and the `micros` field for an interval. pglet has that
arithmetic once, in `pglet_core::types::round_micros_to_precision`, with one wrapper per
type. Tested on both engines:

| statement | both engines | what a naive half-up on the fraction gives |
| --- | --- | --- |
| `'2020-01-02 03:04:05.6789'::timestamp(3)` | `2020-01-02 03:04:05.679` | same |
| `'1999-12-31 23:59:59.9995'::timestamp(3)` | `1999-12-31 23:59:59.999` | `2000-01-01 00:00:00` |
| `'2000-01-01 00:00:00.0005'::timestamp(3)` | `2000-01-01 00:00:00.001` | same |
| `'1969-07-20 20:17:40.0005'::timestamp(3)` | `1969-07-20 20:17:40` | `…40.001` |

Ties go **away from zero**, not to even. `'00:00:00.0025'::time(3)` is `.003` on both
engines, where half-to-even would give `.002`, and `'-0.0015 secs'::interval(3)` is
`-00:00:00.002`. An **interval** rounds only its `micros` field. Months and days are a
*range* spec's job, and a bare `interval(p)` is `INTERVAL_FULL_RANGE`, so
`'1 day 1.5 secs'::interval(0)` is `1 day 00:00:02` on both. A `time` carry **doesn't**
wrap: `'23:59:59.9999'::time(3)` is `24:00:00` on both, which `Value::Time`'s raw `i64` can
represent.

**`p = 6` is a recorded modifier whose rounding does nothing**, which is different from a
bare `timestamp` that has no modifier. 18.6 records `atttypmod = 6` for the first and `-1`
for the second, and `datetime_precision` reads `6` for both. A `p` **above** 6 is clamped
to 6 rather than refused. 18.6 warns (`TIMESTAMP(9) precision reduced to maximum allowed,
6`) and records `6`. pglet doesn't reproduce the warning, because its notice channel hangs
off a `QueryResult` the type-name reader can't reach.

**One number, and it must fit in `int32`.** 18.6's grammar spells both forms with a single
`Iconst` (`TIMESTAMP '(' Iconst ')' opt_timezone`, `ConstInterval '(' Iconst ')'`), so
`timestamp(3,1)`, `timestamp(3.5)`, `timestamp(-1)` and `timestamp(99999999999999)` are
each `42601 syntax error` there. Each is `42601` here too, with pglet's own messages.
`timestamp(2147483647)` does fit, and clamps.

**The precision goes in three different places in a written-out name.**
`docs/deparse.md` § A constant prints in its type's own spelling is where that rule lives
(`cast_type_spelling`). The canonical name the parser produces puts it **last**
(`time with time zone(3)`, so `decompose_type_name`'s cut at the first `(` can't read a
tz-time as a `time`). The dump and `Portable` rendering put it after a one-word name
(`timestamp(3)`). The `Catalog` rendering puts it where `format_type` does, **inside** the
words (`timestamp(3) without time zone`).

**Residual: a rounding carry out of range is an error here and a value in 18.6.** It's one
of the pglet-only refusals in § The date and timestamp domain.
`'9999-12-31 23:59:59.9999'::timestamp(3)` carries into year 10000, which 18.6 can
represent and `OffsetDateTime` can't, so pglet raises `22008 timestamp out of range`
(`ERRCODE_DATETIME_VALUE_OUT_OF_RANGE`, the code 18.6 itself uses when a timestamp leaves
its range) where 18.6 returns `10000-01-01 00:00:00`. It's loud and stricter than 18.6, and
it never silently keeps an unrounded value.

## `timestamptz` text encoding

`timestamptz` has no distinct value (it's stored as a UTC `timestamp`). Only the **column**
`TypeOid` carries the tz/no-tz distinction, so the wire **text** encoder is
column-type-aware (`value_to_text_with_oid`): a `timestamptz` column renders with the UTC
`+00` offset (`… 12:30:45.123+00`), and a plain `timestamp` without one. The offset
matters. A text-protocol driver (node-postgres, the fuz_app matrix) reparses an offset-less
value in its own local zone and silently shifts the instant. pglet is always UTC, the same
as Postgres at `TimeZone=UTC`. **The session's zone and styles are fixed, and a statement
that would change one is refused** (`0A000`) rather than accepted and ignored: `TimeZone`
is `UTC`, `DateStyle` is `ISO, MDY`, and `IntervalStyle` is `postgres`. `SET`,
`SET TIME ZONE`, `set_config` and the native server's startup packet all go through one
check (`catalog::commands`' `check_fixed_guc`, whose doc comment lists the spellings it
handles), and a reset (`DEFAULT`, `SET TIME ZONE LOCAL`, `RESET`) restores pglet's value.
18.6 acts on each of these: after `SET TIME ZONE 'America/New_York'`,
`'2024-01-01 03:00+00'::timestamptz::date` is `2023-12-31` there (tested). Real zones are
`EDGES.md` row 69's open residual. The binary encoder gets the offset right by
construction (the OID tells the driver it's UTC), so the differential fixtures' binary
path can't see this. A `-- @text` fixture (`types/timestamptz_text.sql`) covers it.

**An array's elements follow the same rule, at the declared element type.** A
`timestamptz[]` renders each finite element with `+00` through the same
`value_to_text_with_oid`, which renders an array using its declared type's element type
rather than the element type the `Value::Array` records (`ARRAY[timestamptz '…']` records
`timestamp`). Tested on both engines: the simple-query and extended text formats,
`COPY TO`, the dump, a `::text` / `CAST(… AS varchar)`, and `concat` and `concat_ws`
(which read each argument's declared type, so a scalar `timestamptz` argument keeps its
`+00` too). The wasm host's JS array carries it as well. Without it, the dump's `COPY`
would load each element in the restoring session's zone, which shifts the instant under
`psql -f` at `America/New_York` (tested). Covered by
`types/timestamptz_array_text_offset.sql` (its text leg),
`copy/timestamptz_array_offset_to.sql` and the `test_dump` round-trip, whose Postgres
restore runs at a non-UTC zone.

**The declared element type matters in two more places.** A cast of a `timestamptz[]` to
`text[]` or `varchar[]` renders each element with `+00` (`eval_cast`), as 18.6's
per-element `timestamptz_out` does. And a **binary-format** array's header names the
result column's element type when every non-`NULL` element is sent as that type, and the
element type the value records otherwise (`value_to_binary_with_oid`, `value_sends_as`;
`EDGES.md` row 36). A `timestamptz[]` column's `Value::Timestamp` elements are sent as
`timestamptz`, so an `ARRAY[timestamptz '…']` result's header says 18.6's `1184` (not
`1114`), and an `ARRAY['a'::varchar]` result's says `1043` (not `25`). The element bytes
are the same either way, and the fixture harness's binary leg decodes an array by the
column's element type, so the server unit test
`binary_array_header_carries_the_columns_element_type` checks the header over a raw
`DataRow`. Each was tested on both engines. A bare `$N` whose type the client declared at
`Parse` is a result column of that type, so a `timestamptz` one renders with `+00`, and an
`ARRAY[$1]` over it gets header `1184` (`docs/param_binding.md` § A declared type is part
of parse analysis).

## Fail-loud gaps — `TIMETZ` and any unrecognized type

`TIMETZ` (`time with time zone`) isn't implemented as a **column type**, since pglet is
UTC-only. Plain `TIME` is real. The `CAST`/`::` path and the DDL column path share **one**
type-name reader (`parser::parse_type_name`) and **one** resolver (`executor`'s
`resolve_type_name`, entered as `resolve_column_type` at both). So a multi-word or
length-bearing spelling (`double precision`, `varchar(255)`) parses on both, and `timetz`
and an unrecognized name each give the same `42704` at a cast and at a column. We tested
this: `42704 type "time with time zone" is not supported` comes back for `timetz(3)`,
`timetz`, `time(3) with time zone` and `time with time zone` alike. The two positions can
still give different errors, because they raise at different points. A column definition
leaves an unread `(` for the caller's `,`-or-`)` check, while a cast resolves inline. So
`json(3)[]` is `42601` at a column and `42704` at a cast, and `bytea(3)[]` is `42601` at a
column and `22023` at a cast (tested; `EDGES.md` row 120's bullet records the split). An
unsupported type returns `Error::UnsupportedType` (SQLSTATE `42704`) instead of silently
coercing to TEXT. That's the correct-or-loud rule applied to a type name. Legacy lenient
coercion is opt-in: a per-connection thread-local toggle
(`planner::set_coerce_unknown_types`, off by default), reachable through
`Database::set_coerce_unknown_types` (embed) and the server's `--coerce-unknown-types` flag.
