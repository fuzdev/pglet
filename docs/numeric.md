# NUMERIC / DECIMAL — an in-house exact decimal

`NUMERIC` (a.k.a. `DECIMAL`) is a real column type for the generic test-fixture
audience — a real schema uses it constantly (money, measurements), even though the
fuz spine's schema uses none. It's backed by an **in-house base-10 bignum**
(`pglet_core`'s `PgNumeric`) with **no external dependency** — the decided posture,
holding the dep-free budget at the cost of some extra code. Differential-tested
byte-for-byte against real Postgres (`tests/fixtures/types/numeric_*.sql`).

## Representation — base-10 bignum (`PgNumeric`)

`PgNumeric` is `NaN` or a finite `NumericVar { negative, digits, weight, dscale }`:

- `digits` — base-10 significant digits (`0..=9`), most-significant first,
  **normalized** (no leading/trailing zeros; empty ⇒ zero).
- `weight` — the power-of-ten place of the first digit (`123.45` → weight 2,
  `0.0045` → weight -3).
- `dscale` — the **display scale**, tracked separately from the digits, so `1.50`
  is `digits=[1,5]`, `weight=0`, `dscale=2` (value `1.5`, shown with two places).

Base-10 (rather than Postgres's base-10000 `NBASE`) keeps parsing, formatting,
rounding, and long-hand arithmetic auditable against `numeric.c`. The base-10000
regroup that Postgres's binary wire format needs happens **only** at that boundary
(`to_wire_bytes` / `recv`, aligning `NBASE` groups on the decimal point).

`NaN` follows Postgres numeric semantics (not IEEE floats): it **equals itself** and
**sorts above every finite value**. Value equality is exact (`1.5` == `1.50`), so
`PgNumeric`'s `PartialEq` is value equality, not structural.

## Scale rules (from the PostgreSQL source, `numeric.c`)

Every operation rounds its exact result to a chosen scale (half-away-from-zero) and
sets `dscale`:

- **add / sub** — `dscale = max` of the operands' (the exact sum always fits).
- **mul** — `dscale = dscale1 + dscale2` (the exact product).
- **div** — `select_div_scale`: at least `MIN_SIG_DIGITS` (16 — Postgres's
  `NUMERIC_MIN_SIG_DIGITS`) significant
  digits, never below either input's scale, clamped to `[0, 1000]`. pglet reproduces
  the weight estimate in `NBASE` (base-10000) units × `DEC_DIGITS` (4) by regrouping
  first (`to_nbase_groups`), so `7/10` gives scale 20 as in Postgres — a naive
  base-10 weight estimate would give 17.

## Type surface

- **Column type.** `NUMERIC`, `NUMERIC(p)`, `NUMERIC(p,s)`, and the `DECIMAL`
  spellings resolve to `TypeOid::Numeric` (OID 1700; array `NumericArray` 1231). The
  `NUMERIC(p,s)` modifier is parsed by `parse_numeric_typmod` and captured on the
  catalog `Column.numeric_typmod`. A `NUMERIC(p,s)[]` spelling captures the same
  modifier at the **element's** width — the canonical name decomposes once, so the
  column is a `numeric[]` whose elements each carry `(p,s)` — and `apply_typmod` walks
  the elements, rounding `'{1.25,2.5}'` to `{1.3,2.5}` and refusing a bare `1.25` with
  `42804`, as 18.6 does (`EDGES.md` row 120, closed).
- **Store-time enforcement.** A `NUMERIC(p,s)` value rounds to `scale` and
  precision-overflows loud (`22003`, "numeric field overflow") via
  `PgNumeric::apply_typmod`, applied by the single `coerce_column_value` finalizer
  that every write path storing a non-NULL value (INSERT / UPDATE / `ON CONFLICT DO
  UPDATE` / COPY / DEFAULT / a foreign key's `ON UPDATE` action) funnels through — so
  coercion + typmod can't drift between paths. A `DELETE`'s `ON DELETE SET NULL` writes
  its NULL without it — a value the finalizer returns unchanged.
  An `ON UPDATE CASCADE` into a `numeric(5,2)` child rounds the parent's new key, as
  18.6's action `UPDATE` does (`fk/update_cascade_numeric_typmod_child_rounds.sql`).
- **Comparison / ordering** are exact — never routed through `f64` (which loses
  precision). A numeric-vs-int comparison promotes the int to `PgNumeric` (exact); a
  numeric-vs-float compares via the numeric's `f64` value (Postgres casts
  numeric→float8 for that mixed operator). Backs `=`, `<`/`>`, `ORDER BY`,
  `GROUP BY`, `DISTINCT`.
- **Index key.** `encode_order_key` produces an order-preserving byte key (a class
  byte orders negative < zero < positive < `NaN`; then a sign-flipped `weight`, each
  digit as `digit+1`, and a `0x00` terminator; negatives bitwise-inverted). Equal
  values encode identically, so a `UNIQUE`/PK numeric index collapses `1.5` and
  `1.50` (as Postgres does), and B-tree range scans work.
- **Arithmetic.** `+ − * /` and unary `−` over `PgNumeric`; a numeric operand keeps
  the result numeric (`int + numeric → numeric`), a float operand promotes to
  `float8`. Result-type inference is in `numeric_family_result` (shared by the
  executor's `expr_type_oid` and the planner's param inference).
- **Aggregates.** `SUM`/`AVG` over `numeric` are exact `numeric`, and so are
  `sum(bigint)` and `avg` over `int2`/`int4`/`int8`, as in 18.6: integers are added in
  an `i128` and `numeric`s in a `PgNumeric`, and `avg` divides the exact sum by the count
  through `PgNumeric::div`, so its scale is `select_div_scale`'s (`avg` over `1, 2` is
  `1.5000000000000000`). An integer argument takes the `numeric` answers only where its
  type is certain, a verdict the coercion pass records on the aggregate once
  (`planner::ArgCertainty`); `compute_aggregate` picks these accumulators by the type
  `aggregate_type_of` answers from it, the call both protocols' descriptor walks make, so
  the value is the type both declare. A table's column is certain, and so is a derived
  relation's whose defining expression the pass typed as written — a derived table's or
  `WITH` query's computed column, a `VALUES` list's, a set operation's
  ([coercion.md](coercion.md) § A computed branch is typed where its type is certain;
  `EDGES.md` row 2 has the uncertain members).
  `sum(int4)`, `sum`/`avg` over `real`/`float8`, and `sum`/`avg` over `interval` agree,
  a `DISTINCT` one adding its values in the type's sort order as 18.6 does — except a
  plain one standing beside a `DISTINCT` aggregate, which adds in input order where
  18.6's planner may sort its input (`EDGES.md` row 297; see
  [compatibility.md](compatibility.md)).
- **Casts.** text/int ↔ numeric (exact), numeric ↔ float (the lossy double cast),
  numeric → int (half-away rounding, loud `22003` on overflow). A cast-level
  modifier (`x::numeric(10,2)`, `x::varchar(n)`) is **enforced** — it rides a
  persisted `Expr::CastTypmod` and applies the same `apply_typmod` the column form
  uses (numeric rounds + `22003`; varchar truncates, per the explicit-cast rule).
- **Params.** A bound `$N::numeric` types from column context, decodes the binary
  `numeric_recv` form, and round-trips.
- **`NUMERIC(p, s)` scale** may be **negative** (`numeric(5,-2)` rounds to hundreds
  on store) — the type-name parser accepts the signed modifier, and the precision
  budget is `precision - scale` integer digits (`0.00123::numeric(2,5)` overflows).
  Negative declared scale and scale > precision are PostgreSQL 15+ forms (earlier
  versions required 0 ≤ scale ≤ precision).

## Math functions

`abs` / `ceil` / `ceiling` / `floor` / `round` / `trunc` are exact over `NUMERIC`
(the `Expr::MathFn` IR, shared result-type inference in `math_fn_result_type`), as
are `%` (modulo — exact truncating integer division, sign follows the dividend),
`^` (integer-exponent via repeated squaring; `float8` wins when either operand is
float), `sqrt` (Newton with an exact closed-form ½-ulp final-digit check), and
`sign`. Digit-group underscores (`1_000`, `1_000.5`, `1_0e1_0`) are accepted in
numeric **literals** (Postgres 16+ lexing; trailing junk after a literal fails loud
`42601`) — and only there: a `_`, or a `0x`/`0o`/`0b` prefix, inside a *string* read as
a numeric or an integer is `22P02` here and a value on 18.6 (`EDGES.md` row 189).
`EXTRACT(field FROM …)` also joins the `NUMERIC`-result family — it returns a
`NUMERIC` with Postgres's per-field display scale (scale 6 for `second`/`epoch`,
3 for `milliseconds`, 0 for the integer fields, a long fractional for `julian`),
matching PG; the `date_part(text, …)` spelling returns `float8` instead:

- `abs(x)` preserves the argument's type and display scale (`abs(-1.50) = 1.50`);
  an integer `abs` overflow (`abs(int4 MIN)`) fails loud (`22003`).
- `ceil`/`floor` round to an integer toward ±∞; `round(x [, s])` rounds a `NUMERIC`
  `x` half-away-from-zero to `s` (default 0) places; `trunc(x [, s])` truncates toward
  zero. The two-argument `round(v, s)` / `trunc(v, s)` form exists only for a
  `NUMERIC` value (`s` is an integer; Postgres has no
  `round(double precision, integer)`).
- A float argument computes in `float8`, and so does a one-argument integer call of
  `ceil` / `floor` / `round` / `trunc` (Postgres resolves `ceil(int)` and `round(int)` to
  the `double precision` overload:
  `pg_typeof(ceil(5))` is `double precision` there). A NULL argument yields NULL.
  `round(double precision)` is Postgres's `dround`, C's `rint`: a tie rounds to the even
  integer, so `round(2.5::float8)` is `2` and `round(-0.5::float8)` is `-0`, where
  `round(numeric)` rounds the tie half away from zero (`round(2.5)` is `3`).
  Guards: `functions/round_float8_ties_to_even.sql` and its control
  `functions/round_numeric_ties_away.sql`.

## `CASE` / `COALESCE` branch unification

When a `CASE`/`COALESCE` result set mixes a `NUMERIC` branch with an int/float
**literal** branch (`COALESCE(price, 0)`), the literal is coerced to `NUMERIC`, so
the selected value always carries the projection's reported `NUMERIC` type (else a
literal fallback would surface with its own OID — a wire type mismatch). int/text →
numeric is always representable, so no narrowing overflow.

## Decimal literals are `NUMERIC` (Postgres constant typing)

A number literal with a decimal point / exponent, or an integer too wide for `i64`,
is a `Literal::Numeric` carrying its parsed `PgNumeric` — matching Postgres, where
`1.50` is `numeric` (scale 2), not `float8`. This is load-bearing: routing a literal
through `f64` would lose the trailing-zero scale (`1.50` → `1.5`), diverging from
Postgres on the display scale and on downstream arithmetic. A plain integer literal
that fits `i64` stays an integer.

A literal or string past 18.6's numeric-format caps — the weight, the display scale, or an
exponent whose digits pass `PG_INT32_MAX / 2` — is `22003 value overflows numeric format`
on both engines (`SELECT 1e131073`, `SELECT '1e131073'::numeric`, `SELECT 0e1073741824`;
`SELECT 0e1073741823` is `0`). A literal's refusal is deferred to the statement's end, so
a grammar error outranks it as on 18.6, and it outranks a binding error that 18.6 raises
first (`EDGES.md` row 188).

## Known gaps

- **Cast-level typmod** (`x::numeric(10,2)`, `x::varchar(n)`) is **enforced** — a
  cast whose target carried a modifier lowers to `Expr::CastTypmod` and applies the
  same `apply_typmod` the column form uses, so `1.5::numeric(3)` yields `2` and
  `'abcd'::varchar(3)` yields `'abc'`. The `varchar(n)`/`char(n)` **column** length is
  enforced too — a `Value::Text` over the limit truncates on an explicit cast but
  on a **column store** truncates only when the excess is all blanks, else `22001`
  (Postgres's `isExplicit` split). The length rides the catalog
  `Column.varchar_length` (parenless `char` = `char(1)` at a declared position and no
  length at a typed constant — `docs/parser.md` § Known lexical / parse divergences
  (*A typed constant is a cast*); parenless `varchar` unlimited at both). Residual gaps:
  `char(n)` does not `bpchar`-blank-pad (a `char(3)` stores `'a'` as `'a'`, not `'a  '`)
  and its `22001` message says "character varying" (pglet keeps no char-vs-varchar bit).
  A length **outside** the type's window — below 1, or above 10 485 760 — is not a
  gap: it is 18.6's `22023 length for type {char,varchar} must be at least 1` /
  `… cannot exceed 10485760`, raised where `anychar_typmodin` raises it, at the cast,
  the typed constant, a column definition and `ADD COLUMN` alike, since one
  `resolve_type_name` serves all four. The same refusal covers `numeric`'s own window
  (`numeric(0)`, `numeric(1001)`, `numeric(5,±1001)`, `numeric(5,2,3)`) and the `int4`
  coercion every `numeric` component goes through first; the full member list, the two
  grammar layers, the refusal order and the residuals are
  `docs/coercion.md` § A modifier the type refuses, and `EDGES.md` row 124 is its
  record.
  Two more consequences of the no-char-bit model: a `char(n)` column **reports wire
  OID 1043 (varchar), not 1042 (bpchar)** (there is no `TypeOid::Bpchar`), and
  `char(n)` **comparison is not trailing-blank-insensitive**
  (`'ab' = 'ab '::char(5)` → pglet false, PG true). What makes the divergence reachable
  is that the value carries trailing blanks the type does not strip, since pglet never
  pads — within the limit at a `char(n)` column or cast, and at **any** width through a
  parenless `char` *constant*, which carries no limit at all (`char 'ab ' = char 'ab'`
  is `f` here and `t` there). Every
  measured row of the divergence, in both directions — the four descriptions, the
  comparison, `octet_length`, `concat_ws`, a `PRIMARY KEY` that accepts the
  duplicate 18.6 refuses on the padded key, and the constant position's own cells,
  against the neighbours that agree because
  they go through bpchar→text — is `EDGES.md` row 95, which also states why closing it
  is a larger change than a fix.
- **Infinity.** `NaN` is supported; `±Infinity` (Postgres 14+) parses loud
  (unsupported) — no consumer need yet.
- **Fractional numeric exponent.** `2 ^ 0.5` over numerics fails loud (`0A000`) —
  the ln/exp bignum lift is deferred; a float operand computes in `float8`.
