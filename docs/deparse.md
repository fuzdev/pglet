# Deparsing a stored expression

`crates/pglet_sql/src/deparse.rs` is the single `Expr` deparser. It renders a persisted
planner expression back to SQL for six consumers, in **two modes**, because the two groups
of consumers want different strings for the same stored expression.

## The rule

A stored expression has two renderings, and which one a surface uses is a correctness
decision. pglet stores the expression **as written**. 18.6's `pg_attrdef` /
`pg_constraint` / `pg_index` hold the tree *after parse analysis*: every literal is typed,
every cast to the operand's own type is folded away, and an implicit coercion node is
inserted wherever one was needed. So a faithful rendering of pglet's tree and 18.6's
`pg_get_expr` of its own tree are different strings for the same source, and one renderer
can't produce both.

- **`RenderMode::Portable`** renders pglet's tree faithfully. Its contract is
  `parse(render(e)) == e`, meaning *the identical variant*, which is stricter than
  "evaluates the same". That's because the planner's partial-index implication check
  compares stored expressions structurally, so a restore that changed the shape would
  silently change which queries a restored index serves. Its consumers are `pglet::dump`
  and the DDL-time render probe in `executor::ddl` (`cook_column_default`,
  `validate_stored_expr_renders`).
- **`RenderMode::Catalog`** renders what `pg_get_expr` would print **for the tree 18.6
  would have stored**. Its consumers are the four introspection surfaces:
  `information_schema.columns.column_default`, `information_schema.check_constraints.
  check_clause`, `pg_get_constraintdef`, and `pg_indexes.indexdef`. Nothing re-parses
  those, which is the only reason this mode is allowed to spell coercions pglet never
  stored. It carries a `LeafScope`, because an *unknown* literal's rendering depends on
  the type its position resolved it to. A `DEFAULT` names no column and passes
  `PlannerScope::none`; a CHECK or index predicate passes its own relation.

The two modes share **one match** rather than being two renderers. Every `Expr` variant is
classified in one exhaustive arm list, so a new variant is a compile error instead of
silently inheriting "decline". The modes differ only inside the arms where 18.6's tree and
pglet's differ. `constraint_def` takes the mode as a parameter for the same reason: it's
both the dump's constraint renderer and `pg_get_constraintdef`.

## What parse analysis does that the deparser has to reconstruct

The rules below come from `get_const_expr`, `get_coercion_expr` and `get_func_expr` in
`src/backend/utils/adt/ruleutils.c` in the PostgreSQL source, plus `coerce_type` /
`coerce_to_target_type` from `parse_coerce.c`. Each rule here was also tested against
18.6, not just read off the source, and the fixtures for each are in
`tests/fixtures/catalog/`. **The list is what our sweeps over `CHECK` and `DEFAULT` shapes
reached, not a closed set.** If a rule is missing from this page, that tells you about the
sweeps, not about 18.6.

### A constant prints in its type's own spelling (`get_const_expr`, `showtype = 0`)

| type | rule | measured |
|---|---|---|
| `bool` | `true` / `false`, never labelled | `DEFAULT (NOT true)` → `(NOT true)` |
| `unknown` | quoted, never labelled | `DEFAULT concat('a','b')` → `concat('a', 'b')` |
| `int4`, non-negative | bare | `DEFAULT 5` → `5` |
| `int4`, negative | `'-1'::integer`, so the text re-parses as a constant and not as `-` applied to one | `DEFAULT -1` → `'-1'::integer` |
| an integer outside `int4` | the `default:` arm: quoted and labelled whatever the sign | `DEFAULT 2147483648` → `'2147483648'::bigint` |
| `numeric`, float-looking (`isdigit(extval[0]) && strcspn(extval, "eE.") != strlen(extval)`) | bare | `DEFAULT 1.5` → `1.5` |
| `numeric`, otherwise | `'…'::numeric` | `DEFAULT -1.5` → `'-1.5'::numeric`; `DEFAULT 1e3` → `'1000'::numeric` |
| `NULL` | always labelled when the type is known | `CHECK (a <> NULL)` → `(a <> NULL::integer)` |
| everything else | `'…'::type`, in `format_type`'s spelling | `DEFAULT '{}'::jsonb` → `'{}'::jsonb` |

A *numeric* literal's own type isn't the column's, but a *string* literal's is.
`DEFAULT -1` reads `'-1'::integer` on an `int2`, `int4` and `int8` column alike, and
`DEFAULT -1.5` reads `'-1.5'::numeric` on a `float4` and a `float8` one. The literal is
typed before the assignment cast to the column type, and `pg_get_expr` hides that implicit
cast. pglet stores the *coerced* value, so `information_schema`'s literal renderer has to
recover the width from the value's magnitude instead of reading it off the `Value`
variant. A **string** literal is `unknown` until `coerce_to_target_type` types it *to the
column's own type*, so it goes the other way: `int8 DEFAULT '007'` is `'7'::bigint`,
`int2 DEFAULT '7'` is `'7'::smallint`, `bool DEFAULT 't'` is `true`. These are two arms of
one function. If you take the numeric arm as the rule, a string default gets labelled
where it should fold.

The cast **suffix** uses `format_type`'s spelling, which isn't the DDL spelling a dump
needs: `character varying` vs `varchar`, `timestamp with time zone` vs `timestamptz`,
`time without time zone` vs `time`, `timestamp without time zone` vs `timestamp`. That's
why `deparse::catalog_type_name` and `deparse::type_oid_sql` are two functions.

Where a **modifier** goes in that suffix is a third decision, and `cast_type_spelling`
makes it for both modes. There are **two axes**.

The first is the `[]`. A modifier belongs to the **element**, so an array target spells it
*inside* the suffix: `character varying(3)[]` in `Catalog` and `varchar(3)[]` in
`Portable`, never `character varying[](3)` or `varchar[](3)`. The wrong order doesn't just
differ from 18.6. It doesn't re-parse on either engine, so a dump containing one never
reloads.

The second is **where inside the element's own name** the modifier goes, and that depends
on the *type*, not on the shape of the name. `character varying(3)` and `numeric(4,1)` put
the modifier after the whole name. The three multi-word datetime spellings put it after
the **first word**: `timestamp(3) without time zone`, `timestamp(3) with time zone`,
`time(3) without time zone`, and `interval(3)`, which is one word anyway. We tested each
on 18.6 through `pg_get_constraintdef`, and the array form through `format_type`
(`timestamp(3) without time zone[]`: modifier inside the words, `[]` outside them).
`catalog_type_name_with_modifier` does that split, and it's `Catalog`-only. `Portable`'s
names are one word each (`timestamp(3)`, `timestamptz(3)`, `time(3)`), which is why they
re-parse. `deparse`'s `every_modifier_carrying_type_has_a_decided_modifier_position` pins
both columns of the table, so a new modifier-carrying type can't fall into either half by
default.

### A cast is often not there at all (`coerce_type`, then `get_coercion_expr`)

There are three shapes. The first is a fact about parse analysis rather than a rendering
rule:

1. **`coerce_type` returns its argument untouched when the argument's type already *is*
   the target**, so 18.6 stores no node: `DEFAULT gen_random_uuid()::uuid` reads
   `gen_random_uuid()`, `DEFAULT 5::int4` on an `int4` column reads `5`, and
   `CHECK (s::text = 'x')` over a `text` column reads `(s = 'x'::text)`. The test
   **includes the typmod**, and pglet's doesn't. `coerce_type` returns the argument when
   type *and* typmod both match, so over an `s varchar(3)` column 18.6 elides
   `s::varchar(3)` (`btree (s)` in `indexdef`, `((s)::text <> …)` in
   `pg_get_constraintdef`) and keeps `s::varchar(5)`. `render_cast` keeps a typmod'd cast
   unconditionally and prints `(((s)::character varying(3)))`. We tested both. The
   divergence is open, and it's in the scalar spelling, not the array one.

   **It affects the four temporal types**, whose casts carry a typmod. Over an
   `a timestamp(3)` column, tested on both engines at `pg_get_constraintdef`,
   `check_clause` and `indexdef`'s key and predicate: pglet prints
   `((a)::timestamp(3) without time zone = a)` where 18.6 elides to `(a = a)`, and
   `btree (((a)::timestamp(3) without time zone))` where 18.6 prints `btree (a)`. The
   neighbouring spellings agree. A cast whose `p` differs from the column's
   (`a::timestamp(6)`, `b::timestamptz(1)`, `c::time(1)`, `d::interval(1)`) is a node 18.6
   keeps, and pglet prints it byte-identically. pglet's test compares the type alone, so it
   disagrees with 18.6 exactly where the typmods differ from what a type-only test
   assumes. pglet keeps a cast 18.6 elides when the two typmods **match**, and elides one
   18.6 keeps when only pglet's typmod is absent (`a::timestamp` over an `a timestamp(3)`
   column reads `(a = a)` here and `((a)::timestamp without time zone = a)` there, also
   tested). Fixing it needs the *operand's* typmod, which `LeafScope` doesn't carry.
2. **The operand is a constant of the target type**: `get_coercion_expr` prints it with
   `showtype = -1` and **no parens**, then appends the label: `'5'::text`,
   `1.5::numeric(10,2)`. In pglet's tree the operand is an *unknown* literal that this
   cast types, so every string-literal operand takes this arm.
3. **Anything else** takes the parenthesized form: `DEFAULT 5::int8` on an `int4` column
   reads `(5)::bigint`, and `CHECK (a::text = 'x')` reads `((a)::text = 'x'::text)`.

A **top-level implicit cast is hidden** and a nested one is shown. That's
`get_func_expr`'s `showimplicit`, which is false from `pg_get_expr` and true for every
operator and function argument: `int8 DEFAULT 5::int4` reads `5`, while
`numeric DEFAULT 2.5 * 2` reads `(2.5 * (2)::numeric)`. pglet stores no implicit
coercions at all, so it reproduces the first and not the second. See § What is recorded,
not reproduced.

### An unknown literal takes the type its position resolved it to

A string literal is `unknown` until operator or function resolution types it, and 18.6
stores the typed `Const`. `deparse::operand_type` is the walk that reproduces this. It
types a string literal and a bare `NULL` as `unknown` (`None`), and delegates everything
else to `Expr::infer_type_with`, passing itself as that walk's operand hook so the
unknown-ness propagates. `Expr::infer_type` can't answer this one question: it types every
string literal `text`, which is right for pglet's evaluator and wrong here.
`d = '2024-01-01'` deparses as `'2024-01-01'::date` on 18.6 *precisely because* the
literal wasn't text when the operator was resolved.

`None` is always safe on this walk. It means "no resolution", and every caller falls back
to the unlabelled spelling for `None`: pglet's own spelling, never an invented cast. These
are the positions we tested that resolve, each checked separately (treat the list as what
our corpus reached, not as everything 18.6 does):

- **A binary operator whose chosen operator takes the same type on both sides**: `=`,
  `<>`, `<`, `<=`, `>`, `>=`, `IS DISTINCT FROM`, `IS NOT DISTINCT FROM`, `@>`, `<@`. The
  unknown takes the sibling's type, and a pair of unknowns takes `text`
  (`select_common_type`'s all-unknown rule). `Varchar` maps to `Text`, matching 18.6,
  which relabels the *column* instead (`v = 'x'` over a `varchar(10)` reads
  `((v)::text = 'x'::text)`; pglet renders the literal half but not the relabel).
- **The pattern operators**, which are `text ~~ text` whatever the left operand is.
- **`||`**, when the sibling is a string or itself unknown. With an array or `jsonb`
  sibling Postgres picks a different operator, so this declines instead of guessing.
- **The jsonb key accessors** `->`, `->>`, `?`, whose right operand is `text`.
- **A `text` argument position of a string function**, checked against
  `pg_get_function_arguments` on 18.6 for each name rather than assumed. The padding and
  splitting functions take an `integer` in the middle, `current_setting` takes a `boolean`
  second argument, and `substring` / `date_trunc` are overloaded on their later
  arguments, so those positions decline.
- **`ARRAY[…]`'s elements and `COALESCE`'s arguments**. These take the position's element
  type when it has one, and `select_common_type` over the members otherwise. They decline
  when two known members disagree, since this renderer doesn't model that case's
  coercions.
- **A `CASE`'s branch results**, by the same `select_common_type` rule and with the same
  decline: `CASE WHEN (a = 1) THEN 'y' ELSE 'n' END` reads `THEN 'y'::text … ELSE
  'n'::text`, while an `int`-branched one stays bare. The **test-expression** form
  resolves its `WHEN` values against the operand (`CASE a WHEN 1 THEN 'y' …`). A
  conditionless `WHEN` is a boolean predicate, not a common-type position. No
  differential fixture can check this, because `pg_get_expr` indents a `CASE` over four
  lines (`EDGES.md` row 91), so a unit test guards it instead. The same resolution
  supplies the type of the `ELSE NULL` that 18.6 stores when none is written.
- **`concat_ws`'s first argument**, which is `text` in every `pg_proc` overload while the
  variadic ones are `"any"`: `concat_ws('-', s, 'x')` reads `concat_ws('-'::text, s, 'x')`,
  with the later literals unlabelled. `concat` is `"any"` throughout and resolves nothing.
- **`date_part`'s field argument**, `text` in every overload: `date_part('year', t)` reads
  `date_part('year'::text, t)`. Its `EXTRACT` twin follows a different rule (see below):
  there the field isn't an argument at all.

**`Varchar` folds to `Text` in the operator rule but not in the common-type one**, and each
placement is where testing put it. Over a `varchar(10)` column, 18.6 renders `v = 'x'` as
`((v)::text = 'x'::text)`, so the literal is *text*. But it renders `v IN ('x','y')` as
`((v)::text = ANY ((ARRAY['x'::character varying, 'y'::character varying])::text[]))`,
where `select_common_type` picked *varchar* (the left operand comes first and wins the
tie) and the array is cast afterwards. pglet doesn't store either outer cast
(`EDGES.md` row 90). What it can get right is which type the elements carry, and the two
positions disagree about that.

**A quantified `ANY`/`ALL`'s array is rendered without pushing the left operand's type down
into its elements.** That looks like an omission, but it's deliberate. On 18.6,
`s = ANY (ARRAY['x','y'])` over `text` reads `ARRAY['x'::text, 'y'::text]`, which the
common-type rule over the members already produces. Over a `varchar(10)` column the
elements stay `text` too, and the *column* is relabelled. Over a `numeric` column with
`int` members, the **whole array** is cast (`ANY ((ARRAY[1, 2])::numeric[])`) rather than
its elements. So the left operand's type would be right in the one case the members
already handle, and wrong in the two they don't. And the same predicate's `IN` spelling
casts each element instead (`ARRAY[(1)::numeric, (2)::numeric]`), so those aren't one rule
either.

### Four node identities 18.6 has and pglet's parser does not

- **There is no `IS NOT DISTINCT FROM` node**, and against a bare `NULL` there's no
  `DistinctExpr` either. `transformAExprDistinct` builds the `DistinctExpr` and wraps the
  negated spelling in a `NOT`, so `CHECK (a IS NOT DISTINCT FROM 1)` reads back
  `CHECK ((NOT (a IS DISTINCT FROM 1)))`. **The positive polarity agrees only when
  neither operand is an undecorated `NULL`.** The function's *first* step is to
  short-circuit to `make_nulltest_from_distinct` when `exprIsNullConstant` holds for
  either side, so `a IS NOT DISTINCT FROM NULL` is stored as a `NullTest` and reads
  `(a IS NULL)`, and `a IS DISTINCT FROM NULL` reads `(a IS NOT NULL)`. Three details
  matter, and we tested each. `exprIsNullConstant` checks for an `A_Const` carrying
  `isnull`, so a **cast** NULL is a `TypeCast` and keeps the `DistinctExpr` spelling
  (`a IS NOT DISTINCT FROM NULL::int` → `(NOT (a IS DISTINCT FROM NULL::integer))`). The
  **right** operand is checked first, so `NULL IS NOT DISTINCT FROM NULL` tests the left
  one. And the argument is deliberately **not coerced**, so no type is pushed into it
  (`'x' IS NOT DISTINCT FROM NULL` reads `('x' IS NULL)`, with the literal unlabelled).
  pglet's parser draws the bare-versus-cast line in the same place: a bare `NULL` and a
  parenthesized `(NULL)` stay `Expr::Literal(Literal::Null)`, while `NULL::int` is an
  `Expr::Cast`, confirmed through the `Portable` dump. So this is a rendering rule and
  doesn't change the tree. The one residual is the both-bare case: 18.6 prints
  `(NULL::unknown IS NULL)`, and pglet, which models no `unknown` type name, prints
  `(NULL IS NULL)`. `Portable` keeps the written keyword, since that's what pglet's own
  parser reads back to the same variant.
- **A function whose name is a keyword is quoted.** `generate_function_name` runs the name
  through `quote_identifier`, and `substring` is a `col_name_keyword`, so
  `substring(s, 1, 2)` reads `"substring"(s, 1, 2)`. It's the only name in pglet's
  function tables that both **renders through `generate_function_name`** and has that
  property, and that qualifier is the whole claim. `position` never renders by name (it
  uses the `POSITION(x IN y)` special form), nor does the `::regclass` cast. `greatest` /
  `least` / `nullif` aren't `pg_proc` calls at all (next bullet), and every other name is
  unreserved.
- **Three of the five variadic names are keyword constructs, not calls.** `greatest` /
  `least` build a `MinMaxExpr` and `nullif` a `NullIfExpr`, and `get_rule_expr` prints
  each in **uppercase** (`GREATEST(a, b)`, `LEAST(a, b)`, `NULLIF(a, b)`), the same way it
  prints `COALESCE(`, whose arm is the control. `concat` and `concat_ws` *are* `pg_proc`
  entries and stay lowercase. The unknown-resolving rule splits them the same way.

  `GREATEST` / `LEAST` are `select_common_type` positions over all their arguments
  (`transformMinMaxExpr`), so they use the common-type rule with `Varchar` **not**
  folded. `LEAST('a', s)` over `text` reads `LEAST('a'::text, s)`, and `LEAST(v, 'x')`
  over `varchar(10)` reads `LEAST(v, 'x'::character varying)`. That's the whole node when
  it sits in a position that asks nothing more of it (`… IS NOT NULL`). Under a `text`
  comparison, 18.6 relabels the result too
  (`(LEAST(v, 'x'::character varying))::text <> ''::text`), and pglet renders no such
  outer cast: row 90 again.

  `NULLIF` resolves the `=` **operator** over its pair (`transformAExprNullIf`), so its
  unknown takes the sibling's type with `Varchar` folded. The two engines produce
  different strings here: over a `varchar(10)` column, `NULLIF(v, 'x')` renders
  `NULLIF(v, 'x'::text)` **in pglet** and `NULLIF((v)::text, 'x'::text)` **on 18.6**
  (tested on both). This rule gets the sibling-typed literal right; the operand's relabel
  is `EDGES.md` row 90's residual. A pair whose known members disagree still declines
  (`GREATEST(n, 1)` over `numeric` reads `GREATEST(n, (1)::numeric)` on 18.6). That's
  `EDGES.md` row 90's class, and it's the only part of this family that belongs to it.
- **`EXTRACT`'s field is not an argument.** `ruleutils.c`'s `F_EXTRACT_TEXT_*` arm prints
  the field `Const`'s text through `quote_identifier`, not as a labelled literal. A field
  that is a plain lowercase identifier, and not a keyword `quote_identifier` quotes, comes
  back bare. Any other field comes back as a quoted identifier with each `"` doubled:
  `EXTRACT('YEAR' FROM t)` reads `EXTRACT("YEAR" FROM t)`, `EXTRACT('select' FROM t)`
  reads `EXTRACT("select" FROM t)`, `EXTRACT('my"x' FROM t)` reads
  `EXTRACT("my""x" FROM t)`, and `EXTRACT('year' FROM t)` reads `EXTRACT(year FROM t)`.
  **The written case is kept only where the grammar kept it.** The bare-identifier form
  `EXTRACT(EPOCH FROM t)` is downcased by the *grammar* on both engines, so both render
  `EXTRACT(epoch FROM t)`. A string constant or a quoted identifier is stored verbatim, so
  `EXTRACT('EPOCH' FROM t)` and `EXTRACT("Hour" FROM t)` keep their case inside the
  quotes. pglet's comma spelling with a string-constant field, `extract('year', t)`,
  builds the same node and renders the same way. With any other field expression it's
  the third arm, and renders back to that comma form, which 18.6's grammar can't spell.
  The shapes 18.6 can spell are pinned at each catalog surface by
  `catalog/extract_field_quoting_*.sql`. `Portable` keeps the quoted literal, since
  that's the spelling pglet's parser reads back to the same node whatever the string
  holds.

### `LIKE` is grammar, not an operator name

The parser builds `~~` / `!~~` / `~~*` / `!~~*` and `ruleutils.c` prints the operator, so
`CHECK (s LIKE 'x%')` reads back `CHECK ((s ~~ 'x%'::text))` on 18.6. We tested all four.
The portable rendering keeps the keyword, because pglet's parser has no `~~` operator, so a
dump emitting one wouldn't reload.

### An `IN` list is not a node 18.6 has

`transformAExprIn` (`src/backend/parser/parse_expr.c`) rewrites the list during parse
analysis, and `ruleutils.c` deparses whatever it built. **The rewrite has more than one
shape**, and a fix that assumes it's always `= ANY (ARRAY[…])` gets it wrong:

- Elements are split by `contain_vars_of_level`. One that holds a column stays an operand
  of the boolean chain, and the rest are candidates for the array.
- The array is built only when **more than one** non-column element remains, so
  `a IN (1)` is `(a = 1)` and `a IN (1, b)` is `((a = 1) OR (a = b))`.
- `NOT IN` is the same transformation under `<>`, which flips the quantifier to `ALL`
  and the chain to `AND`: `(a <> ALL (ARRAY[1, 2, 3]))`, `(a <> 1)`,
  `((a <> b) AND (a <> c))`.
- The array comes **first** in the chain, then the column elements in list order:
  `a IN (b, c, 1, 2)` reads `(((a = ANY (ARRAY[1, 2])) OR (a = b)) OR (a = c))`.
- **The chain is left-folded pairwise, not joined n-ary.**
  `makeBoolExpr(…, list_make2(result, cmp))` runs inside `transformAExprIn`'s `foreach`,
  so an n-member chain is n-1 nested binary `BoolExpr`s, and `ruleutils.c` prints a paren
  per fold. With two members a left fold and a flat join look the same, so only a chain
  of three or more tells them apart: a flat join renders `a IN (1, 2, b, d)` one paren
  short.

That last rule is the **opposite** of `render_bool_chain`'s, and the two don't interact.
`makeOrExpr` flattens a *written* `a = 1 OR a = 2 OR …` at grammar time, immediately, but
an `IN` is still an `A_Expr` at that point, not a `BoolExpr`, so it never joins a
neighbour's flatten. We tested both directions: `a IN (b, c) OR a = 1 OR a = 2` is a
**three**-arg `OR` whose first arg is the `IN`'s own nested pair, and
`(a IN (b, c)) OR (a IN (1, 2, b, c))` stays two args.

One rule is **declined rather than approximated**. `select_common_type` may pick a type
that none of the operands has, and 18.6 then wraps every one of them
(`a IN (1, 2.5)` reads `((a)::numeric = ANY (ARRAY[(1)::numeric, 2.5]))`). pglet stores
no such coercions, so when the elements don't already share the left operand's type, the
rendering falls back to the written `IN`. We'd rather diverge with the written spelling
than print a shape neither engine holds.

**The transformation is done at render time, not by normalising the plan, and that choice
matters.** Normalising the stored plan instead would change what every `InList` reader in
the evaluator and the index-probe path sees. It would also make the dump emit
`= ANY (ARRAY[…])`, which re-parses to `Expr::AnyAll`, a different variant. That breaks
the round-trip invariant that keeps a restored partial index serving the same queries. So
`Expr::AnyAll` and `Expr::InList` render to byte-identical text in `Catalog` mode (as they
must, since 18.6 stores the same node for both spellings) and to their own written
spellings in `Portable` mode.

### Two spellings a catalog rendering may take and a portable one may not

- `nextval('s'::regclass)`, with the `::regclass` suffix. A dump has to go back through
  the general expression grammar, which refuses that spelling: `parse_cast_target` lowers
  the `regclass` cast to a `StringFn` that `sequence_name_from_arg`'s look-through
  doesn't match, so the result is pglet's own `0A000`. So `Portable` emits the bare
  literal, and `Catalog` emits the suffix 18.6 prints.
- `'\x00'` rather than `E'\x00'`. `quote_string_literal`'s `E'…'` form is a *dump* rule:
  it makes a backslash mean the same byte under either `standard_conforming_strings`
  setting. Nothing re-parses a catalog view, and 18.6 prints `simple_quote_literal`'s
  plain form there.

## What is recorded, not reproduced

These were found on 18.6 in the `CHECK` and `DEFAULT` sweeps this rule was derived from
(`EDGES.md` has the rows):

- **Implicit numeric / integer widening shown in an operator or array argument.**
  `CHECK (n > 1)` over a `numeric` column reads `(n > (1)::numeric)` on 18.6 and `(n > 1)`
  in pglet. `DEFAULT 2.5 * 2` reads `(2.5 * (2)::numeric)`. `a IN (1, 2.5)` and
  `a IN (1, 1000000000000)` are the same class, reaching the `IN` fallback above. pglet
  stores no implicit coercion nodes, so reproducing these would mean modelling
  `select_common_type` and the assignment-cast insertion at render time.
- **`CASE`'s pretty-printing, and only that.** `pg_get_expr` runs with
  `PRETTYFLAG_INDENT`, so a `CASE` comes back over four lines with a four-space indent,
  and pglet renders it on one. The *contents* agree: the branch results resolve their
  unknowns, and an ELSE-less `CASE` renders 18.6's implicit `ELSE NULL::<result type>`
  (`transformCaseExpr` supplies a NULL `defresult`; we tested several result types).
  `Portable` adds no such `ELSE`, because emitting one would re-parse to a different
  variant.
- **A `varchar` operand's relabel to `text`**: `((v)::text = 'x'::text)` vs
  `(v = 'x'::text)`.
- **An enum comparison's label.** A stored enum value is a `Value::Text`, with the enum
  name kept on the *column*, so a literal compared against an enum column resolves to
  `text` in pglet and to the enum's own type on 18.6. The `DEFAULT` position does have the
  enum name, because `render_column_default` holds the column.
- **A value the type's output function would normalise.** The catalog rendering prints
  the literal verbatim, where 18.6 prints `typoutput`'s text. So
  `DEFAULT '{"a":1}'::jsonb` reads `'{"a":1}'::jsonb` in pglet and `'{"a": 1}'::jsonb` on
  18.6, `DEFAULT ' 2024-01-01 '::date` keeps its spaces, and `time DEFAULT '10:00'` reads
  `'10:00'::time without time zone` vs `'10:00:00'::…`. The types whose output *is*
  reproduced are `bool`, `int4`, `int2`, `int8` and `numeric`. For the first three, the
  fold decides whether a label is printed at all (`'007'::int4` reads `7`, `'t'::bool`
  reads `true`). `int2`/`int8` fold under a label that's always printed (`'007'` on a
  `bigint` column is `'7'::bigint`, `get_const_expr`'s `default:` arm).
  **That fold has a single implementation, `deparse::render_catalog_string_const`, used
  from every position including `information_schema.columns.column_default`.** A second
  copy at one position would diverge there while agreeing everywhere else. For a *string*
  `DEFAULT`, the resolving type is the **column's** (`coerce_to_target_type`'s target),
  which is why `int8 DEFAULT '007'` is `'7'::bigint` and not `7`: only `int4` takes
  `get_const_expr`'s bare arm.
- **The two `COERCE_SQL_SYNTAX` spellings.** 18.6 remembers *how a call was written* and
  prints it back: `SUBSTRING(s FROM 1 FOR 2)`, `TRIM(BOTH FROM s)`,
  `POSITION(('a'::text) IN (s))`. pglet normalises each to one node at parse time, so it
  doesn't store which spelling was written. It reproduces the function-call spelling of
  the same call (`"substring"(s, 1, 2)`). Fixing this needs a stored spelling bit, not a
  rendering rule.
- **`DEFAULT +5` is folded by pglet's `DEFAULT` classifier** (`parser::ddl`'s
  `column_default_from_expr` sees through the `Expr::UnaryPlus` the parser keeps and
  stores the bare literal), while 18.6 keeps the unary-plus `OpExpr`. So `DEFAULT +5`
  reads `5` in pglet and `(+ 5)` on 18.6. Anywhere else the wrapper is stored and renders
  as `(+ …)` (`deparse`'s `Expr::UnaryPlus` arm). This comes from a storage decision, not
  a rendering one.
