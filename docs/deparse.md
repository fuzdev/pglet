# Deparsing a stored expression

`crates/pglet_sql/src/deparse.rs` is the one `Expr` deparser. It renders a persisted
planner expression back to SQL for six consumers, and it does so in **two modes**,
because the two groups of consumers want two different strings for the same stored
expression.

## The rule

**A stored expression has two renderings, and which one a surface takes is a
correctness decision.** pglet stores the expression **as written**; 18.6's
`pg_attrdef` / `pg_constraint` / `pg_index` hold the tree *after parse analysis* — every
literal typed, every cast to the operand's own type already folded away, an implicit
coercion node inserted wherever one was needed. So a faithful rendering of pglet's tree
and 18.6's `pg_get_expr` of its own tree are different strings for the same source, and
one renderer cannot be both.

- **`RenderMode::Portable`** renders pglet's tree faithfully. Its contract is
  `parse(render(e)) == e` — not "evaluates the same", *the identical variant* — because
  the planner's partial-index implication check compares stored expressions
  structurally, so a restore that changed the shape would silently change which queries
  a restored index serves. Its consumers are `pglet::dump` and the DDL-time render probe
  in `executor::ddl` (`cook_column_default`, `validate_stored_expr_renders`).
- **`RenderMode::Catalog`** renders what `pg_get_expr` would print **for the tree 18.6
  would have stored**. Its consumers are the four introspection surfaces —
  `information_schema.columns.column_default`, `information_schema.check_constraints.
  check_clause`, `pg_get_constraintdef`, and `pg_indexes.indexdef`. Nothing re-parses
  those, and that is the whole licence this mode has: it may spell coercions pglet never
  stored. It carries a `LeafScope` — a `DEFAULT` names no column and passes
  `PlannerScope::none`, a CHECK or index predicate passes its own relation — because an
  *unknown* literal's rendering depends on the type the position resolved it to.

The two modes are **one match**, not two renderers. Every `Expr` variant is
classified in one exhaustive arm list, so a new variant is a compile error rather than a
silent inheritance of "decline"; the modes differ only inside the arms where 18.6's tree
and pglet's genuinely part. `constraint_def` takes the mode as a parameter for the same
reason: it is the dump's constraint renderer *and* `pg_get_constraintdef`.

## What parse analysis does that the deparser has to reconstruct

The rules below are `get_const_expr`, `get_coercion_expr` and `get_func_expr` from
`src/backend/utils/adt/ruleutils.c` in the PostgreSQL source, plus `coerce_type` /
`coerce_to_target_type` from `parse_coerce.c`, and each rule stated here was measured
against 18.6 rather than read off alone; the fixtures naming each are in
`tests/fixtures/catalog/`. **The list is what measured sweeps over `CHECK` and `DEFAULT` shapes
reached, not a closed set**; a rule's absence from this page is evidence about the
sweeps, never about 18.6.

### A constant prints in its type's own spelling (`get_const_expr`, `showtype = 0`)

| type | rule | measured |
|---|---|---|
| `bool` | `true` / `false`, never labelled | `DEFAULT (NOT true)` → `(NOT true)` |
| `unknown` | quoted, never labelled | `DEFAULT concat('a','b')` → `concat('a', 'b')` |
| `int4`, non-negative | bare | `DEFAULT 5` → `5` |
| `int4`, negative | `'-1'::integer` — so the text re-parses as a constant, not as `-` applied to one | `DEFAULT -1` → `'-1'::integer` |
| an integer outside `int4` | the `default:` arm, quoted and labelled whatever the sign | `DEFAULT 2147483648` → `'2147483648'::bigint` |
| `numeric`, float-looking (`isdigit(extval[0]) && strcspn(extval, "eE.") != strlen(extval)`) | bare | `DEFAULT 1.5` → `1.5` |
| `numeric`, otherwise | `'…'::numeric` | `DEFAULT -1.5` → `'-1.5'::numeric`; `DEFAULT 1e3` → `'1000'::numeric` |
| `NULL` | always labelled when the type is known | `CHECK (a <> NULL)` → `(a <> NULL::integer)` |
| everything else | `'…'::type`, in `format_type`'s spelling | `DEFAULT '{}'::jsonb` → `'{}'::jsonb` |

**A *numeric* literal's own type is not the column's, and a *string* literal's is.**
`DEFAULT -1` reads `'-1'::integer` on an `int2`, an `int4` and an `int8` column alike,
and `DEFAULT -1.5` reads `'-1.5'::numeric` on a `float4` and a `float8` one: the literal
is typed before the assignment cast to the column type, and that cast is an implicit one
`pg_get_expr` hides. pglet stores the *coerced* value, so `information_schema`'s literal
renderer has to put the width back from the value's magnitude rather than read it off
the `Value` variant. A **string** literal is `unknown` until `coerce_to_target_type`
types it *to the column's own type*, so it goes the other way: `int8 DEFAULT '007'` is
`'7'::bigint`, `int2 DEFAULT '7'` is `'7'::smallint`, `bool DEFAULT 't'` is `true`. The
two halves are one function's two arms, and reading the first for the rule is what makes
a string default label where it should fold.

The cast **suffix** spelling is `format_type`'s, which is not the DDL spelling a dump
needs — `character varying` against `varchar`, `timestamp with time zone` against
`timestamptz`, `time without time zone` against `time`, `timestamp without time zone`
against `timestamp`. `deparse::catalog_type_name` and `deparse::type_oid_sql` are two
functions for that reason.

Where a **modifier** goes in that suffix is a third decision, and `cast_type_spelling`
is the one place it is made for both modes. It has **two axes**, not one.

The first is the `[]`: a modifier is the **element's**, so an array target spells it
*inside* the suffix — `character varying(3)[]` in `Catalog` and `varchar(3)[]` in
`Portable`, never `character varying[](3)` or `varchar[](3)`. The wrong order is not
merely unlike 18.6; it re-parses on neither engine, so a dump carrying one never
reloads.

The second is **where inside the element's own name**, and there the answer is the
*type's*, not the name's shape. `character varying(3)` and `numeric(4,1)` take the
modifier after the whole name; the three multi-word datetime spellings take it after the
**first word** — `timestamp(3) without time zone`, `timestamp(3) with time zone`,
`time(3) without time zone`, and `interval(3)`, which is one word anyway. Each measured
on 18.6 through `pg_get_constraintdef`, and the array form off `format_type`
(`timestamp(3) without time zone[]`: modifier inside the words, `[]` outside them).
`catalog_type_name_with_modifier` is that split, and it is `Catalog`-only:
`Portable`'s names are one word each — `timestamp(3)`, `timestamptz(3)`, `time(3)` —
which is what makes them re-parse, and `deparse`'s
`every_modifier_carrying_type_has_a_decided_modifier_position` pins both columns of the
table so a new modifier-carrying type cannot join either half by default.

### A cast is often not there at all (`coerce_type`, then `get_coercion_expr`)

Three shapes, and the first is a parse-analysis fact rather than a rendering rule:

1. **`coerce_type` returns its argument untouched when the argument's type already *is*
   the target**, so 18.6 stores no node: `DEFAULT gen_random_uuid()::uuid` reads
   `gen_random_uuid()`, `DEFAULT 5::int4` on an `int4` column reads `5`, and
   `CHECK (s::text = 'x')` over a `text` column reads `(s = 'x'::text)`. The test is on
   the **typmod too**, and pglet's is not: `coerce_type` returns the argument when type
   *and* typmod both match, so over an `s varchar(3)` column 18.6 elides `s::varchar(3)`
   (`btree (s)` in `indexdef`, `((s)::text <> …)` in `pg_get_constraintdef`) and keeps
   `s::varchar(5)`, while `render_cast` keeps a typmod'd cast unconditionally and prints
   `(((s)::character varying(3)))`. Both measured; the divergence is open and is the
   scalar spelling's, not the array one's.

   **It reaches the four temporal types**, whose casts carry a typmod. Over an `a timestamp(3)` column, measured on both engines at
   `pg_get_constraintdef`, `check_clause` and `indexdef`'s key and predicate: pglet
   prints `((a)::timestamp(3) without time zone = a)` where 18.6 elides to `(a = a)`,
   and `btree (((a)::timestamp(3) without time zone))` where 18.6 prints `btree (a)`.
   The neighbouring spellings agree — a cast whose `p` differs from the column's
   (`a::timestamp(6)`, `b::timestamptz(1)`, `c::time(1)`, `d::interval(1)`) is a node
   18.6 keeps and pglet prints byte-identically. What the test
   compares is the type alone, so it disagrees with 18.6 at exactly the spellings where
   the typmods differ from what a type-only test assumes: pglet keeps a cast 18.6 elides
   when the two typmods **match**, and elides one 18.6 keeps when only pglet's is absent
   (`a::timestamp` over an `a timestamp(3)` column reads `(a = a)` here and
   `((a)::timestamp without time zone = a)` there — measured).
   Closing it needs the *operand's* typmod, which `LeafScope` does not carry.
2. **The operand is a constant of the target type**: `get_coercion_expr` prints it with
   `showtype = -1` and **no parens**, then appends the label — `'5'::text`,
   `1.5::numeric(10,2)`. In pglet's tree the operand is an *unknown* literal that this
   cast is what types, so every string-literal operand takes this arm.
3. **Anything else** takes the parenthesized form: `DEFAULT 5::int8` on an `int4` column
   reads `(5)::bigint`, `CHECK (a::text = 'x')` reads `((a)::text = 'x'::text)`.

A **top-level implicit cast is hidden** and a nested one is shown (`get_func_expr`'s
`showimplicit`, false from `pg_get_expr` and true for every operator and function
argument): `int8 DEFAULT 5::int4` reads `5` while `numeric DEFAULT 2.5 * 2` reads
`(2.5 * (2)::numeric)`. pglet stores no implicit coercions at all, so it reproduces the
first and not the second — see § What is recorded, not reproduced.

### An unknown literal takes the type its position resolved it to

A string literal is `unknown` until operator or function resolution types it, and 18.6
stores the typed `Const`. `deparse::operand_type` is the walk that reproduces this: it
types a string literal and a bare `NULL` as `unknown` (`None`) and delegates everything
else to `Expr::infer_type_with`, supplying itself as that walk's operand hook so the
unknown-ness propagates. It is the one thing `Expr::infer_type` cannot answer — that
walk types every string literal `text`, which is right for pglet's evaluator and wrong
here, because `d = '2024-01-01'` deparses `'2024-01-01'::date` on 18.6 *precisely
because* the literal was not text when the operator was resolved.

`None` is always safe on this walk: it means "no resolution", and every caller's
fallback for `None` is the unlabelled spelling — pglet's own, never an invented cast.
The positions measured to resolve, each with its own measurement (treat the list as the
corpus's reach rather than as 18.6's):

- **A binary operator whose chosen operator is same-type on both sides** — `=`, `<>`,
  `<`, `<=`, `>`, `>=`, `IS DISTINCT FROM`, `IS NOT DISTINCT FROM`, `@>`, `<@`: the
  unknown takes the sibling's type, and a pair of unknowns takes `text`
  (`select_common_type`'s all-unknown rule). `Varchar` maps to `Text`, matching 18.6,
  which relabels the *column* instead (`v = 'x'` over a `varchar(10)` reads
  `((v)::text = 'x'::text)` — pglet renders the literal half and not the relabel).
- **The pattern operators**, which are `text ~~ text` whatever the left operand is.
- **`||`**, when the sibling is a string or itself unknown. Over an array or `jsonb`
  sibling Postgres picks a different operator, so this declines rather than guesses.
- **The jsonb key accessors** `->`, `->>`, `?`, whose right operand is `text`.
- **A `text` argument position of a string function**, read off
  `pg_get_function_arguments` on 18.6 for each name rather than assumed: the padding and
  splitting functions take an `integer` in the middle, `current_setting` a `boolean`
  second argument, and `substring` / `date_trunc` are overloaded on their later
  arguments, so those positions decline.
- **`ARRAY[…]`'s elements and `COALESCE`'s arguments**, which take the position's
  element type when it has one and `select_common_type` over the members otherwise —
  and decline when two known members disagree, which is the case whose coercions this
  renderer does not model.
- **A `CASE`'s branch results**, by the same `select_common_type` rule and with the same
  decline: `CASE WHEN (a = 1) THEN 'y' ELSE 'n' END` reads `THEN 'y'::text … ELSE
  'n'::text` while an `int`-branched one stays bare. The **test-expression** form
  resolves its `WHEN` values against the operand (`CASE a WHEN 1 THEN 'y' …`); a
  conditionless `WHEN` is a boolean predicate and no common-type position at all. No
  differential fixture can hold this — `pg_get_expr` indents a `CASE` over four lines
  (`EDGES.md` row 91) — so the guard is a unit test. The same resolution supplies the
  type of the `ELSE NULL` 18.6 stores when none is written.
- **`concat_ws`'s first argument**, `text` in every `pg_proc` overload while the variadic
  ones are `"any"`: `concat_ws('-', s, 'x')` reads `concat_ws('-'::text, s, 'x')`, the
  later literals unlabelled. `concat` is `"any"` throughout and resolves nothing.
- **`date_part`'s field argument**, `text` in every overload:
  `date_part('year', t)` reads `date_part('year'::text, t)`. Its `EXTRACT` twin is a
  different rule — see below, the field is not an argument at all there.

**`Varchar` folds to `Text` in the operator rule and not in the common-type one**, and
each placement is where its own measurement puts it. Over a `varchar(10)` column, 18.6
renders `v = 'x'` as `((v)::text = 'x'::text)` — the literal is *text* — but renders
`v IN ('x','y')` as
`((v)::text = ANY ((ARRAY['x'::character varying, 'y'::character varying])::text[]))`,
where `select_common_type` picked *varchar* (the left operand is first and so wins the
tie) and the array is cast afterwards. Both outer casts are ones pglet does not store
(`EDGES.md` row 90); what it can get right is which type the elements carry, and the two
positions disagree about that.

**A quantified `ANY`/`ALL`'s array is rendered with no element type pushed down from the
left operand**, which looks like an omission and is not. Measured on 18.6:
`s = ANY (ARRAY['x','y'])` over `text` reads `ARRAY['x'::text, 'y'::text]`, which the
common-type rule over the members already produces; over a `varchar(10)` column the
elements stay `text` too, with the *column* relabelled; and over a `numeric` column with
`int` members the **whole array** is cast (`ANY ((ARRAY[1, 2])::numeric[])`) rather than
its elements. So the left operand's type would be right in the one case the members
already answer and wrong in the two they do not — and it is the same predicate's `IN`
spelling that casts each element instead (`ARRAY[(1)::numeric, (2)::numeric]`), so those
are not one rule either.

### Four node identities 18.6 has and pglet's parser does not

- **There is no `IS NOT DISTINCT FROM` node** — and against a bare `NULL` there is no
  `DistinctExpr` either. `transformAExprDistinct` builds the `DistinctExpr` and wraps
  the negated spelling in a `NOT`, so `CHECK (a IS NOT DISTINCT FROM 1)` reads back
  `CHECK ((NOT (a IS DISTINCT FROM 1)))`. **The positive polarity agrees only when
  neither operand is an undecorated `NULL`**: the function's *first* act is to short-circuit to
  `make_nulltest_from_distinct` when `exprIsNullConstant` holds of either side, so
  `a IS NOT DISTINCT FROM NULL` is stored as a `NullTest` and reads `(a IS NULL)`, and
  `a IS DISTINCT FROM NULL` reads `(a IS NOT NULL)`. Three details are load-bearing and
  each is measured: `exprIsNullConstant` tests for an `A_Const` carrying `isnull`, so a
  **cast** NULL is a `TypeCast` and keeps the `DistinctExpr` spelling
  (`a IS NOT DISTINCT FROM NULL::int` → `(NOT (a IS DISTINCT FROM NULL::integer))`); the
  **right** operand is tested first, so `NULL IS NOT DISTINCT FROM NULL` tests the left
  one; and the argument is deliberately **not coerced**, so no type is pushed into it
  (`'x' IS NOT DISTINCT FROM NULL` reads `('x' IS NULL)`, the literal unlabelled).
  pglet's parser draws the bare-versus-cast line in the same place — a bare `NULL` and a
  parenthesized `(NULL)` stay `Expr::Literal(Literal::Null)` while `NULL::int` is an
  `Expr::Cast`, confirmed through the `Portable` dump — so this is a rendering rule, not
  a tree change. Its one residual is the both-bare case: 18.6 prints
  `(NULL::unknown IS NULL)` and pglet, modelling no `unknown` type name, prints
  `(NULL IS NULL)`. `Portable` keeps the written keyword — it is what pglet's own parser
  reads back to the same variant.
- **A function whose name is a keyword is quoted.** `generate_function_name` runs the
  name through `quote_identifier`, and `substring` is a `col_name_keyword`, so
  `substring(s, 1, 2)` reads `"substring"(s, 1, 2)`. It is the only member of pglet's
  function tables that **renders through `generate_function_name`** and has that
  property, and the qualifier is the whole content of the claim: `position` never
  renders by name (it takes the `POSITION(x IN y)` special form), nor does the
  `::regclass` cast, `greatest` / `least` /
  `nullif` are not `pg_proc` calls at all (next bullet), and every remaining name is
  unreserved.
- **Three of the five variadic names are keyword constructs, not calls.** `greatest` /
  `least` build a `MinMaxExpr` and `nullif` a `NullIfExpr`, and `get_rule_expr` prints
  each **uppercase** — `GREATEST(a, b)`, `LEAST(a, b)`, `NULLIF(a, b)` — exactly as it
  prints `COALESCE(`, whose arm is the control. `concat` and
  `concat_ws` *are* `pg_proc` entries and stay lowercase. The unknown-resolving rule
  splits them the same way: `GREATEST` / `LEAST` are `select_common_type` positions over
  all their arguments (`transformMinMaxExpr`), so they take the common-type rule with
  `Varchar` **not** folded — `LEAST('a', s)` over `text` reads `LEAST('a'::text, s)` and
  `LEAST(v, 'x')` over `varchar(10)` reads `LEAST(v, 'x'::character varying)`, which is
  the whole node in a position that asks nothing more of it (`… IS NOT NULL`); under a
  `text` comparison 18.6 relabels the result too (`(LEAST(v, 'x'::character varying))::text
  <> ''::text`) and pglet renders no such outer cast, row 90 again — while
  `NULLIF` resolves the `=` **operator** over its pair (`transformAExprNullIf`), so its
  unknown takes the sibling's type with `Varchar` folded — and the two engines are not
  the same string here: over a `varchar(10)` column `NULLIF(v, 'x')` renders
  `NULLIF(v, 'x'::text)` **here** and `NULLIF((v)::text, 'x'::text)` on **18.6**
  (measured on both). The sibling-typed literal is what this rule buys; the operand's
  relabel is `EDGES.md` row 90's residual. A pair whose known members disagree still declines
  (`GREATEST(n, 1)` over `numeric` reads `GREATEST(n, (1)::numeric)` there), which is
  `EDGES.md` row 90's class and is the only part of this family that belongs to it.
- **`EXTRACT`'s field is not an argument.** `ruleutils.c`'s `F_EXTRACT_TEXT_*` arm
  prints the field `Const`'s text through `quote_identifier`, not as a labelled literal:
  a field that is a plain lowercase identifier and no keyword `quote_identifier` quotes
  comes back bare, and any other comes back a quoted identifier with each `"` doubled —
  `EXTRACT('YEAR' FROM t)` reads `EXTRACT("YEAR" FROM t)`, `EXTRACT('select' FROM t)`
  `EXTRACT("select" FROM t)`, `EXTRACT('my"x' FROM t)` `EXTRACT("my""x" FROM t)`,
  `EXTRACT('year' FROM t)` `EXTRACT(year FROM t)`. **The case it keeps is the written
  one only where the grammar kept it.** The bare-identifier form `EXTRACT(EPOCH FROM t)`
  is downcased by the *grammar* — on both engines, so both render
  `EXTRACT(epoch FROM t)` — while a string constant or a quoted identifier is stored
  verbatim, so `EXTRACT('EPOCH' FROM t)` and `EXTRACT("Hour" FROM t)` keep their case
  inside the quotes. pglet's comma spelling with a string-constant field,
  `extract('year', t)`, builds the same node and renders the same way; one with any
  other field expression is the third arm and renders back to that comma form, which
  18.6's grammar cannot spell. The shapes 18.6 can spell are pinned at each catalog
  surface by `catalog/extract_field_quoting_*.sql`. `Portable` keeps the quoted literal, which is
  the spelling pglet's parser reads back to the same node whatever the string holds.

### `LIKE` is grammar, not an operator name

The parser builds `~~` / `!~~` / `~~*` / `!~~*` and `ruleutils.c` prints the operator, so
`CHECK (s LIKE 'x%')` reads back `CHECK ((s ~~ 'x%'::text))` on 18.6 — all four
measured. The portable rendering keeps the keyword: pglet's parser has no `~~` operator,
so a dump emitting one would not reload.

### An `IN` list is not a node 18.6 has

`transformAExprIn` (`src/backend/parser/parse_expr.c`) rewrites the list at parse
analysis and `ruleutils.c` deparses whatever it built. **The rewrite is not one shape**,
and taking `= ANY (ARRAY[…])` for the rule is what a fix gets wrong:

- Elements are split by `contain_vars_of_level`: one holding a column stays an operand
  of the boolean chain, the rest are candidates for the array.
- The array is built only when **more than one** non-column element remains — so
  `a IN (1)` is `(a = 1)` and `a IN (1, b)` is `((a = 1) OR (a = b))`.
- `NOT IN` is the same transformation under `<>`, which flips the quantifier to `ALL`
  and the chain to `AND`: `(a <> ALL (ARRAY[1, 2, 3]))`, `(a <> 1)`,
  `((a <> b) AND (a <> c))`.
- The array comes **first** in the chain, then the column elements in list order:
  `a IN (b, c, 1, 2)` reads `(((a = ANY (ARRAY[1, 2])) OR (a = b)) OR (a = c))`.
- **The chain is left-folded pairwise, not joined n-ary.**
  `makeBoolExpr(…, list_make2(result, cmp))` runs inside `transformAExprIn`'s
  `foreach`, so an n-member chain is n-1 nested binary `BoolExpr`s and `ruleutils.c`
  prints a paren per fold. Two members is the one arity at which a left fold and a flat
  join agree, so only a chain of three or more members tells them apart: a flat join
  renders `a IN (1, 2, b, d)` one paren short.

That last rule is the **opposite** of `render_bool_chain`'s, and the two do not meet:
`makeOrExpr` flattens a *written* `a = 1 OR a = 2 OR …` at grammar time, on sight, but
an `IN` is an `A_Expr` at that point and not yet a `BoolExpr`, so it never joins a
neighbour's flatten. Measured, both directions: `a IN (b, c) OR a = 1 OR a = 2` is a
**three**-arg `OR` whose first arg is the `IN`'s own nested pair, and
`(a IN (b, c)) OR (a IN (1, 2, b, c))` stays two args.

One rule is **declined rather than approximated**: `select_common_type` may pick a type
none of the operands has, and 18.6 then wraps every one of them
(`a IN (1, 2.5)` reads `((a)::numeric = ANY (ARRAY[(1)::numeric, 2.5]))`). pglet stores
no such coercions, so when the elements do not already share the left operand's type the
rendering falls back to the written `IN` — the written spelling's divergence, rather than a shape
neither engine holds.

**The transformation is a rendering, not a normalisation, and that choice is the
load-bearing one.** Normalising the stored plan instead would change what every
`InList` reader in the evaluator and the index-probe path sees, and would make the dump
emit `= ANY (ARRAY[…])`, which re-parses to `Expr::AnyAll` — a different variant,
breaking the round-trip invariant that keeps a restored partial index serving the same
queries. `Expr::AnyAll` and `Expr::InList` therefore render to byte-identical text in
`Catalog` mode (as they must: 18.6 stores the same node for both spellings) and to their
own written spellings in `Portable` mode.

### Two spellings a catalog rendering may take and a portable one may not

- `nextval('s'::regclass)` — the `::regclass` suffix. A dump has to go back through the
  general expression grammar, which refuses that spelling — `parse_cast_target` lowers
  the `regclass` cast to a `StringFn` that `sequence_name_from_arg`'s look-through does
  not match, so the answer is pglet's own `0A000`. So `Portable` emits the bare literal
  and `Catalog` emits the suffix 18.6 prints.
- `'\x00'` rather than `E'\x00'`. `quote_string_literal`'s `E'…'` form is a *dump* rule
  — it makes a backslash mean the same byte under either `standard_conforming_strings`.
  Nothing re-parses a catalog view, and 18.6 prints `simple_quote_literal`'s plain form
  there.

## What is recorded, not reproduced

Measured on 18.6 over the `CHECK` and `DEFAULT` sweeps this rule was derived from
(`EDGES.md` carries the rows):

- **Implicit numeric / integer widening shown in an operator or array argument.**
  `CHECK (n > 1)` over a `numeric` column reads `(n > (1)::numeric)` there and `(n > 1)`
  here; `DEFAULT 2.5 * 2` reads `(2.5 * (2)::numeric)`; `a IN (1, 2.5)` and
  `a IN (1, 1000000000000)` are the same class reaching the `IN` fallback above. pglet
  stores no implicit coercion nodes, so reproducing these means modelling
  `select_common_type` and the assignment-cast insertion at render time.
- **`CASE`'s pretty-printing, and that alone.** `pg_get_expr` runs with
  `PRETTYFLAG_INDENT`, so a `CASE` comes back over four lines with a four-space indent;
  pglet renders it on one. Its *contents* agree: the branch results resolve their
  unknowns, and an ELSE-less `CASE` renders 18.6's implicit `ELSE NULL::<result type>`
  (`transformCaseExpr` supplies a NULL `defresult` — measured at several result types).
  `Portable` adds no such `ELSE`; emitting one would re-parse to a different variant.
- **A `varchar` operand's relabel to `text`** — `((v)::text = 'x'::text)` against
  `(v = 'x'::text)`.
- **An enum comparison's label.** A stored enum value is `Value::Text` with the enum
  name beside it on the *column*, so a literal compared against an enum column resolves
  to `text` here and to the enum's own type there. The `DEFAULT` position does carry the
  enum name, because `render_column_default` holds the column.
- **A value the type's output function would normalise.** The catalog rendering prints
  the literal verbatim where 18.6 prints `typoutput`'s text, so `DEFAULT '{"a":1}'::jsonb`
  reads `'{"a":1}'::jsonb` here and `'{"a": 1}'::jsonb` there,
  `DEFAULT ' 2024-01-01 '::date` keeps its spaces, and `time DEFAULT '10:00'` reads
  `'10:00'::time without time zone` against `'10:00:00'::…`. The types whose output *is*
  reproduced are `bool`, `int4`, `int2`, `int8` and `numeric` — for the first three the
  fold decides whether a label is printed at all (`'007'::int4` reads `7`, `'t'::bool`
  reads `true`), and `int2`/`int8` fold under a label that is always printed (`'007'` on
  a `bigint` column is `'7'::bigint`, `get_const_expr`'s `default:` arm).
  **That fold is one implementation, `deparse::render_catalog_string_const`, reached from
  every position including `information_schema.columns.column_default`** — a second copy
  at one position would diverge there while agreeing everywhere else. For a *string* `DEFAULT` the resolving type
  is the **column's**, `coerce_to_target_type`'s target, which is why `int8 DEFAULT '007'`
  is `'7'::bigint` and not `7`: only `int4` takes `get_const_expr`'s bare arm.
- **The two `COERCE_SQL_SYNTAX` spellings.** 18.6 remembers *how a call was written* and
  prints it back: `SUBSTRING(s FROM 1 FOR 2)`, `TRIM(BOTH FROM s)`,
  `POSITION(('a'::text) IN (s))`. pglet normalises each to one node at parse, so the
  written spelling is a bit it does not store; the function-call spelling of the same
  call is the one it reproduces (`"substring"(s, 1, 2)`). Closing this means a stored
  spelling bit, not a rendering rule.
- **`DEFAULT +5` is folded by pglet's `DEFAULT` classifier** (`parser::ddl`'s
  `column_default_from_expr` sees through the `Expr::UnaryPlus` the parser keeps and
  stores the bare literal), where 18.6 keeps the unary-plus `OpExpr`: `DEFAULT +5` reads
  `5` here and `(+ 5)` there. Anywhere else the wrapper is stored and renders `(+ …)`
  (`deparse`'s `Expr::UnaryPlus` arm). A storage decision reaching this surface, not a
  rendering one.
