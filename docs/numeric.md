# NUMERIC / DECIMAL — an in-house exact decimal

`NUMERIC` (also spelled `DECIMAL`) is a full column type for the generic test-fixture
audience. The fuz spine's schema doesn't use it, but real schemas use it all the time
(money, measurements). It's backed by an **in-house base-10 bignum** (`pglet_core`'s
`PgNumeric`) with **no external dependency**. That's a deliberate decision: it keeps the
dep-free budget at the cost of some extra code. It's differential-tested byte-for-byte
against real Postgres (`tests/fixtures/types/numeric_*.sql`).

## Representation — base-10 bignum (`PgNumeric`)

`PgNumeric` is `NaN` or a finite `NumericVar { negative, digits, weight, dscale }`:

- `digits` — base-10 significant digits (`0..=9`), most significant first,
  **normalized** (no leading or trailing zeros; empty means zero).
- `weight` — the power-of-ten place of the first digit (`123.45` → weight 2,
  `0.0045` → weight -3).
- `dscale` — the **display scale**, tracked separately from the digits. So `1.50` is
  `digits=[1,5]`, `weight=0`, `dscale=2` (the value `1.5`, shown with two places).

We use base 10 instead of Postgres's base-10000 `NBASE` so parsing, formatting, rounding
and long-hand arithmetic are easy to check against `numeric.c`. Postgres's binary wire
format needs base-10000 groups, and that regrouping happens **only** at that boundary
(`to_wire_bytes` / `recv`, aligning `NBASE` groups on the decimal point).

`NaN` follows Postgres numeric semantics, not IEEE floats: it **equals itself** and
**sorts above every finite value**. Value equality is exact (`1.5` == `1.50`), so
`PgNumeric`'s `PartialEq` compares values, not structure.

## Scale rules (from the PostgreSQL source, `numeric.c`)

Every operation rounds its exact result to a chosen scale (half away from zero) and sets
`dscale`:

- **add / sub** — `dscale` is the `max` of the operands' scales (the exact sum always
  fits).
- **mul** — `dscale = dscale1 + dscale2` (the exact product).
- **div** — `select_div_scale`: at least `MIN_SIG_DIGITS` (16, Postgres's
  `NUMERIC_MIN_SIG_DIGITS`) significant digits, never below either input's scale,
  clamped to `[0, 1000]`. Postgres estimates the weight in `NBASE` (base-10000) units ×
  `DEC_DIGITS` (4), so pglet regroups first (`to_nbase_groups`) to get the same estimate.
  That's why `7/10` gives scale 20 as in Postgres. A naive base-10 weight estimate would
  give 17.

## Type surface

- **Column type.** `NUMERIC`, `NUMERIC(p)`, `NUMERIC(p,s)` and the `DECIMAL` spellings
  resolve to `TypeOid::Numeric` (OID 1700; array `NumericArray` 1231).
  `parse_numeric_typmod` parses the `NUMERIC(p,s)` modifier, and it's stored on the
  catalog `Column.numeric_typmod`. A `NUMERIC(p,s)[]` spelling stores the same modifier
  at the **element's** width. The canonical name is decomposed once, so the column is a
  `numeric[]` whose elements each carry `(p,s)`. `apply_typmod` walks the elements: it
  rounds `'{1.25,2.5}'` to `{1.3,2.5}` and refuses a bare `1.25` with `42804`, as 18.6
  does (`EDGES.md` row 120, closed).
- **Store-time enforcement.** A `NUMERIC(p,s)` value is rounded to `scale`, and a
  precision overflow fails loud (`22003`, "numeric field overflow"), through
  `PgNumeric::apply_typmod`. It's applied by the single `coerce_column_value` finalizer
  that every write path storing a non-NULL value goes through (INSERT / UPDATE /
  `ON CONFLICT DO UPDATE` / COPY / DEFAULT / a foreign key's `ON UPDATE` action), so
  coercion and typmod can't drift apart between paths. A `DELETE`'s
  `ON DELETE SET NULL` writes its NULL without it, which is fine because the finalizer
  returns NULL unchanged. An `ON UPDATE CASCADE` into a `numeric(5,2)` child rounds the
  parent's new key, as 18.6's action `UPDATE` does
  (`fk/update_cascade_numeric_typmod_child_rounds.sql`).
- **Comparison / ordering** are exact and never go through `f64`, which loses precision.
  A numeric-vs-int comparison promotes the int to `PgNumeric` (exact). A numeric-vs-float
  comparison uses the numeric's `f64` value, because Postgres casts numeric→float8 for
  that mixed operator. This covers `=`, `<`/`>`, `ORDER BY`, `GROUP BY` and `DISTINCT`.
- **Index key.** `encode_order_key` produces an order-preserving byte key. A class byte
  orders negative < zero < positive < `NaN`, followed by a sign-flipped `weight`, each
  digit as `digit+1`, and a `0x00` terminator; negatives are bitwise-inverted. Equal
  values encode identically, so a `UNIQUE`/PK numeric index treats `1.5` and `1.50` as
  the same key (as Postgres does), and B-tree range scans work.
- **Arithmetic.** `+ − * /` and unary `−` over `PgNumeric`. A numeric operand keeps the
  result numeric (`int + numeric → numeric`), and a float operand promotes to `float8`.
  Result-type inference lives in `numeric_family_result`, which the executor's
  `expr_type_oid` and the planner's param inference both use.
- **Aggregates.** `SUM`/`AVG` over `numeric` return exact `numeric`, and so do
  `sum(bigint)` and `avg` over `int2`/`int4`/`int8`, as in 18.6. Integers are added in an
  `i128` and `numeric`s in a `PgNumeric`. `avg` divides the exact sum by the count with
  `PgNumeric::div`, so its scale comes from `select_div_scale` (`avg` over `1, 2` is
  `1.5000000000000000`). An integer argument only gets the `numeric` results where its
  type is certain. The coercion pass decides that once and records it on the aggregate
  (`planner::ArgCertainty`). `compute_aggregate` picks these accumulators by the type
  `aggregate_type_of` returns from that record, which is the same call both protocols'
  descriptor walks make, so the value always has the type both declare. A table's column
  is certain. So is a derived relation's column when the pass typed its defining
  expression as written: a derived table's or `WITH` query's computed column, a `VALUES`
  list's, a set operation's ([coercion.md](coercion.md) § A computed branch is typed
  where its type is certain; `EDGES.md` row 2 has the uncertain cases).
  `sum(int4)`, `sum`/`avg` over `real`/`float8`, and `sum`/`avg` over `interval` match
  18.6. A `DISTINCT` one adds its values in the type's sort order, as 18.6 does. The
  exception is a plain one next to a `DISTINCT` aggregate: it adds in input order, where
  18.6's planner may sort its input (`EDGES.md` row 297; see
  [compatibility.md](compatibility.md)).
- **Casts.** text/int ↔ numeric (exact), numeric ↔ float (the lossy double cast),
  numeric → int (half-away rounding, loud `22003` on overflow). A cast-level modifier
  (`x::numeric(10,2)`, `x::varchar(n)`) is **enforced**. It's kept on a persisted
  `Expr::CastTypmod` and applies the same `apply_typmod` the column form uses (numeric
  rounds and raises `22003`; varchar truncates, per the explicit-cast rule).
- **Params.** A bound `$N::numeric` takes its type from the column context, decodes the
  binary `numeric_recv` form, and round-trips.
- **`NUMERIC(p, s)` scale** can be **negative**: `numeric(5,-2)` rounds to hundreds on
  store. The type-name parser accepts the signed modifier, and the precision budget is
  `precision - scale` integer digits (`0.00123::numeric(2,5)` overflows). Negative
  declared scale and scale > precision are PostgreSQL 15+ forms (earlier versions
  required 0 ≤ scale ≤ precision).

## Math functions

`abs` / `ceil` / `ceiling` / `floor` / `round` / `trunc` are exact over `NUMERIC` (the
`Expr::MathFn` IR, with shared result-type inference in `math_fn_result_type`). So are
`%` (modulo: exact truncating integer division, the sign follows the dividend), `^`
(integer exponent via repeated squaring; `float8` wins when either operand is a float),
`sqrt` (Newton's method with an exact closed-form ½-ulp check on the final digit), and
`sign`.

Digit-group underscores (`1_000`, `1_000.5`, `1_0e1_0`) are accepted in numeric
**literals** (Postgres 16+ lexing; trailing junk after a literal fails loud `42601`).
They're accepted only there. A `_`, or a `0x`/`0o`/`0b` prefix, inside a *string* read as
a numeric or an integer is `22P02` in pglet and a value on 18.6 (`EDGES.md` row 189).

`EXTRACT(field FROM …)` also belongs to the `NUMERIC`-result family. It returns a
`NUMERIC` with Postgres's per-field display scale (scale 6 for `second`/`epoch`, 3 for
`milliseconds`, 0 for the integer fields, a long fraction for `julian`), matching
Postgres. The `date_part(text, …)` spelling returns `float8` instead.

- `abs(x)` keeps the argument's type and display scale (`abs(-1.50) = 1.50`). An integer
  `abs` overflow (`abs(int4 MIN)`) fails loud (`22003`).
- `ceil`/`floor` round to an integer toward ±∞. `round(x [, s])` rounds a `NUMERIC` `x`
  half away from zero to `s` places (default 0). `trunc(x [, s])` truncates toward zero.
  The two-argument `round(v, s)` / `trunc(v, s)` form exists only for a `NUMERIC` value
  (`s` is an integer; Postgres has no `round(double precision, integer)`).
- A float argument computes in `float8`. So does a one-argument integer call of `ceil` /
  `floor` / `round` / `trunc`, because Postgres resolves `ceil(int)` and `round(int)` to
  the `double precision` overload (`pg_typeof(ceil(5))` is `double precision` there). A
  NULL argument gives NULL. `round(double precision)` is Postgres's `dround`, which is C's
  `rint`: a tie rounds to the even integer, so `round(2.5::float8)` is `2` and
  `round(-0.5::float8)` is `-0`. `round(numeric)` rounds a tie half away from zero
  instead (`round(2.5)` is `3`). Fixtures: `functions/round_float8_ties_to_even.sql` and
  its control `functions/round_numeric_ties_away.sql`.

## `CASE` / `COALESCE` branch unification

When a `CASE`/`COALESCE` mixes a `NUMERIC` branch with an int or float **literal** branch
(`COALESCE(price, 0)`), the literal is coerced to `NUMERIC`. That way the selected value
always has the `NUMERIC` type the projection reports. Otherwise a literal fallback would
come back with its own OID, which is a wire type mismatch. int/text → numeric is always
representable, so there's no narrowing overflow.

## Decimal literals are `NUMERIC` (Postgres constant typing)

A number literal with a decimal point or exponent, or an integer too wide for `i64`, is a
`Literal::Numeric` holding its parsed `PgNumeric`. This matches Postgres, where `1.50` is
`numeric` (scale 2) and not `float8`. It matters: sending a literal through `f64` would
lose the trailing-zero scale (`1.50` → `1.5`), so pglet would differ from Postgres on the
display scale and on any arithmetic that follows. A plain integer literal that fits `i64`
stays an integer.

A literal or string past 18.6's numeric-format limits (the weight, the display scale, or
an exponent whose digits pass `PG_INT32_MAX / 2`) is `22003 value overflows numeric
format` on both engines (`SELECT 1e131073`, `SELECT '1e131073'::numeric`,
`SELECT 0e1073741824`; `SELECT 0e1073741823` is `0`). A literal's error is deferred to
the end of the statement, so a grammar error wins over it, as on 18.6. It does win over a
binding error that 18.6 raises first (`EDGES.md` row 188).

## Known gaps

- **Cast-level typmod** (`x::numeric(10,2)`, `x::varchar(n)`) is **enforced**. A cast
  whose target has a modifier lowers to `Expr::CastTypmod` and applies the same
  `apply_typmod` the column form uses, so `1.5::numeric(3)` gives `2` and
  `'abcd'::varchar(3)` gives `'abc'`. The `varchar(n)`/`char(n)` **column** length is
  enforced too. A `Value::Text` over the limit is truncated on an explicit cast, but on a
  **column store** it's truncated only when the excess is all blanks, and otherwise it's
  `22001` (Postgres's `isExplicit` split). The length is stored on the catalog
  `Column.varchar_length`. A parenless `char` is `char(1)` in a declared position and has
  no length in a typed constant (`docs/parser.md` § Known lexical / parse divergences
  (*A typed constant is a cast*)); a parenless `varchar` is unlimited in both.
  Remaining gaps: `char(n)` doesn't blank-pad like `bpchar` (a `char(3)` stores `'a'` as
  `'a'`, not `'a  '`), and its `22001` message says "character varying" (pglet keeps no
  char-vs-varchar bit).
  A length **outside** the type's allowed range (below 1, or above 10 485 760) isn't a
  gap. It's 18.6's `22023 length for type {char,varchar} must be at least 1` /
  `… cannot exceed 10485760`, raised where `anychar_typmodin` raises it: at the cast, the
  typed constant, a column definition and `ADD COLUMN`, since one `resolve_type_name`
  serves all four. The same refusal covers `numeric`'s own range (`numeric(0)`,
  `numeric(1001)`, `numeric(5,±1001)`, `numeric(5,2,3)`) and the `int4` coercion every
  `numeric` component goes through first. The full list of cases, the two grammar
  layers, the refusal order and the residuals are in `docs/coercion.md` § A modifier the
  type refuses, and `EDGES.md` row 124 is its record.
  The no-char-bit model has two more consequences. A `char(n)` column **reports wire
  OID 1043 (varchar), not 1042 (bpchar)** (there's no `TypeOid::Bpchar`), and `char(n)`
  **comparison isn't trailing-blank-insensitive** (`'ab' = 'ab '::char(5)` is false in
  pglet and true in Postgres). You can hit this because pglet never pads, so the value
  keeps trailing blanks the type doesn't strip. That happens within the limit at a
  `char(n)` column or cast, and at **any** width through a parenless `char` *constant*,
  which has no limit at all (`char 'ab ' = char 'ab'` is `f` in pglet and `t` in 18.6).
  `EDGES.md` row 95 lists every case we measured, in both directions: the four
  descriptions, the comparison, `octet_length`, `concat_ws`, a `PRIMARY KEY` that accepts
  the duplicate 18.6 refuses on the padded key, and the constant position's own cases,
  along with the neighbours that agree because they go through bpchar→text. That row
  also explains why closing it is a bigger change than a fix.
- **Infinity.** `NaN` is supported. `±Infinity` (Postgres 14+) fails loud as unsupported
  when parsed, since no consumer needs it yet.
- **Fractional numeric exponent.** `2 ^ 0.5` over numerics fails loud (`0A000`), because
  the ln/exp bignum work is deferred. A float operand computes in `float8`.
