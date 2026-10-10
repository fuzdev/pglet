# SQL parser — pglet's own native, Postgres-only front-end

pglet parses SQL with its own native parser; there is no third-party parser
dependency. SQL text lowers **directly** to the planner IR (`Expr`/`Plan`) with no
intermediate AST, so a parse is also a plan. The code lives in
`crates/pglet_sql/src/parser/` (`mod.rs` entry, `expr.rs` expression engine,
`select.rs`/`dml.rs`/`ddl.rs` statements, `with_recursive.rs` the `WITH RECURSIVE`
list's pre-scan and re-parse, `plpgsql.rs` the `DO $$…$$` block,
`type_name.rs` type-name normalization, `stream.rs` the token cursor; `fuzz.rs` is
a `#[cfg(test)]` fuzz module).

## Why native

- **Expression / partial `ON CONFLICT`.** The spine's grant upsert keys on
  `COALESCE(scope_kind,'') … WHERE revoked_at IS NULL` — an expression + partial
  conflict target, which `sqlparser`'s AST cannot represent (the parser's reason to
  exist); the native parser parses, plans and executes it.
- **Size.** `sqlparser` drags a `recursive → stacker → psm` C-toolchain subtree — the
  single biggest wasm-size lever: replacing it cut the wasm from ~709 KB to ~318 KB
  gzip at the time (−55%); see `TRADEOFFS.md`.

## Design

- **Direct-to-IR (fused).** The parser emits planner `Expr`/`Plan` straight away — no
  separate AST arena (pglet has no formatter, so a second tree buys nothing) and no
  second traversal. Per-construct SQLSTATE (`42601` syntax / `42704` unsupported type
  / `0A000` unsupported construct) is raised at parse time for expression/cast
  constructs; a DDL column type-name passes through and resolves (loud) at plan time.
- **Recursive-descent + Pratt.** Statements parse by recursive descent; expressions
  use a Pratt core — a single `parse_expr_bp(min_bp)` loop over a *pure*
  binary-operator binding-power table, with the keyword-led forms that aren't clean
  binary operators (`NOT`, the `IS …` postfix family, `BETWEEN`/`IN`/`LIKE`, `::`, …)
  as explicit productions. A recursion depth-guard bounds expression nesting, and the
  tree the parser builds is bounded as it is built (`pglet_sql::depth`).
- **Streaming lexer + 2-token cursor.** The lexer is pull-based; `stream.rs` is a
  fixed two-slot lookahead window (SQL is LL-friendly — 1–2 tokens suffice, no
  byte-scan hacks). **One construct needs more, and takes a restore point instead of
  a wider window**: a *multi-word* typed constant (`DOUBLE PRECISION '1.5'`,
  `NUMERIC(5,2) '1.5'`) is a type name of unbounded length followed by a string, and
  every one of its lead words is also a legal bare column name. `TokenStream`'s
  `checkpoint`/`restore` (a cloned lexer cursor plus the two-token window — the
  lexer is a borrowed `&str` and an offset, so the clone is free) lets that one
  production be tried and put back, which is what Postgres's LALR grammar does by
  deferring the reduction — except over a window the reader has **committed** to,
  where a failure is the statement's answer and nothing is put back (the typed-constant
  bullet below states the rule and which lead word it covers). Every other decision
  still fits in the window.
- **Shared lowering.** Statement parsing routes through the same `lower_select` /
  `lower_insert` / `lower_update` / `lower_delete` / `build_call` helpers the planner
  owns, so a query lowers to the canonical plan — index selection, ORDER BY pushdown,
  and aggregate/window classification included.
- **Keyword classification is the parser's job.** The lexer only recognizes
  spellings; reserved-vs-identifier classification (`is_reserved`) lives in the
  parser, so a non-reserved keyword (a column named `sequence`, an alias `AS exists`)
  parses as an identifier.

## Scope

Modern-Postgres-only, sized to the spine's SQL surface: DML (INSERT incl.
expression/partial `ON CONFLICT`; UPDATE incl. a target alias and `UPDATE … FROM
other` joined-source updates; DELETE), DDL (CREATE TABLE/INDEX/TYPE/SCHEMA/SEQUENCE,
ALTER incl. ALTER SEQUENCE, DROP TABLE/INDEX/TYPE/SCHEMA/SEQUENCE, TRUNCATE —
`CREATE`/`DROP SCHEMA` model the
single implicit `public` schema, the migration-reset idiom; `CREATE`/`DROP INDEX`
take `CONCURRENTLY` in `gram.y`'s keyword position, and `CREATE`/`DROP EXTENSION`
parse with their full option tails over a fixed name set — see [ddl.md](ddl.md)
§ Extensions), `COPY … FROM
STDIN`/`TO STDOUT` plus the `pg_dump`-preamble tolerance no-ops
(`GRANT`/`REVOKE`/`COMMENT ON`/`OWNER TO`), a **minimal `DO $$ … $$`** block (the idempotent
`CREATE TYPE` idiom — `BEGIN <stmts>; [EXCEPTION WHEN <cond> THEN <handler>;] END`,
not general PL/pgSQL — its `EXCEPTION` clause is a subtransaction, so a caught error
discards the body), transactions **including savepoints** (`SAVEPOINT n`,
`ROLLBACK TO [SAVEPOINT] n`, `RELEASE [SAVEPOINT] n` — the optional noise word is
itself a legal `ColId`, so `RELEASE savepoint` names one), and full SELECT (joins, derived tables,
FROM-`unnest`, CTEs — including **data-modifying CTEs** (`WITH x AS (UPDATE/DELETE/
INSERT … RETURNING) … FROM x`, lowered to a `Plan::With` + `Plan::CteScan`; the
statement after a `WITH` list is a query or a data-modifying statement, any other a
`42601` at its first token, as `gram.y` has it — the `TABLE` and `MERGE` spellings, which
pglet parses nowhere, included), a
`name(cols)` column list on any of them (carried beside the plan and applied at every
reader, never baked into the body — `docs/scope.md`), the `MATERIALIZED` /
`NOT MATERIALIZED` noise words, and a
**`WITH RECURSIVE`**, its list resolved in dependency order and each recursive CTE a
`Plan::With` entry — window
`OVER`, GROUP BY/HAVING/ORDER BY/LIMIT, `SELECT DISTINCT` / **`DISTINCT ON (…)`**, `CASE`,
joins incl. **`CROSS JOIN`**, comma-join, and **`JOIN … USING`**, and a trailing
row-locking clause — the real `FOR {UPDATE|NO KEY UPDATE|SHARE|KEY SHARE} [OF …]`
grammar plus `FOR READ ONLY` as the clause's **first-and-only** production (`gram.y`
makes the two alternatives, so neither order of `FOR UPDATE` + `FOR READ ONLY` is
legal), honored as a no-op, fail-loud on an invalid strength, and fail-loud (`0A000`,
deferred to the end of the statement's parse — every route, a `DO` body's own
sub-parser included) on a `NOWAIT`/`SKIP LOCKED` wait policy — the one part of the
clause a lock-free engine cannot stand in for). **Set operations** `UNION`/`INTERSECT`/`EXCEPT` (each with optional `ALL`)
parse into a chained `Plan::SetOp` at Postgres precedence (`INTERSECT` binds tighter
than `UNION`/`EXCEPT`, left-associative), with a trailing `ORDER BY`/`LIMIT` binding to
the combined result (`Limit(Sort(SetOp(…)))`). The **scalar special-form grammars**
`EXTRACT(field FROM x)` / `SUBSTRING(x FROM n FOR m)` / `TRIM([BOTH|LEADING|TRAILING] c
FROM x)` / `POSITION(a IN b)`, the three `INTERVAL '…'` input grammars (verbose,
SQL-standard `1-2`, ISO-8601 `P1Y2M3DT4H5M6S`), and `nextval`/`currval`/`setval` as
general expressions (not just in `DEFAULT`) all parse. Large `VALUES` INSERTs parse in a flat iterative loop
(`parse_values_rows`): no per-row recursion, so no stack-overflow risk and no separate
streaming path ([storage.md](storage.md) § Known bottlenecks). Per-feature end-to-end
status lives in [compatibility.md](compatibility.md); this list is the parser's own scope.

## Known lexical / parse divergences

- **An operator is lexed as `scan.l` lexes one, and refused where 18.6 refuses it.**
  Every `op_chars` run is one token (`lexer::TokenKind::Op` where pglet's grammar has no
  kind of its own for it), cut where a `/*` or `--` opens inside it and trimmed of a
  trailing `+`/`-` unless the run holds a character SQL's own operators do not use — so
  `1 =- 1` is `1 = -1` and `2 ^- 1` names the operator `^-`, both as on 18.6. The name
  then reaches **analysis**, where an operator pglet implements nothing for is
  `42883 operator does not exist: <left> <op> <right>` — 18.6's own sentence, byte for
  byte where the operands are literals, casts or calls. The refusal is deferred to the
  statement's end, so a grammar error 18.6's raw parse raises still outranks it
  (`SELECT 1 ! 1 1` is `42601` on both). What diverges — the operators 18.6 has and
  pglet does not, and what the refusal says and outranks — is [compatibility.md](compatibility.md)'s *Operator
  lexing + resolution* row and `EDGES.md` row 184. Guards: `lexer::tests`'
  `the_operator_rules_are_the_reference_scanners` and `tests/fixtures/operators/`.
- **A setting's name is compared case-insensitively after the fold.** 18.6's
  `find_option` matches a GUC name through `guc_name_compare`, which ignores ASCII case
  whatever the quoting, so `SET "Quote_All_Identifiers" = on`, `RESET "Lock_Timeout"`,
  `SHOW "Search_Path"` and `set_config('Client_Encoding', …)` name the settings their
  lower-case spellings name (measured). `Token::name` keeps a quoted spelling verbatim, as the
  identifier rule says, so each reader of a setting's name — `catalog::commands`' `SET`,
  `RESET` and `SHOW` readers, the `set_config` / `current_setting` argument, the startup
  packet's keys and the `-c name=value` / `--name=value` assignments in its `options` —
  lowercases it before any comparison; a reader that did not let a quoted mixed-case name
  walk past every check keyed on the name. A setting whose value pglet decides is decided at
  every one of those doors. The startup packet's `options` is read as 18.6 reads it —
  `pg_split_opts`, then `process_postgres_switches`' `getopt`, clustered switches and switch
  arguments included — and its settings applied in 18.6's order, `options` first, each one
  pglet models checked where it is read, pglet's own refusal (a `FATAL`: `quote_all_identifiers=on` is `0A000`, as
  its `SET` is, and so is a `search_path` naming a schema pglet does not have) judged on the
  value the startup ends with (`pglet_server`'s `startup` module).
  Guards: `tests/fixtures/commands/set_guc_quoted_mixed_case_*.sql`,
  `show_guc_quoted_mixed_case_name.sql`, `set_local_guc_quoted_upper_case_loud.sql`, and
  `pglet_server`'s `startup_reads_its_settings_as_18_6_does` and `startup` module tests.
- **A `--` comment ends at a line feed or a carriage return**, as `scan.l`'s does
  (`non_newline` is `[^\n\r]`), so SQL after a lone carriage return on a comment's line
  is read: `SELECT 1 --c<CR>+ 1` is `2` on both engines and `DELETE FROM t -- c<CR>WHERE a
  = 1` deletes one row. The statement splitter and the transaction-exit classifier take
  their tokens from the same lexer, so they agree. Guards: `lexer::tests`'
  `a_line_comment_ends_at_a_carriage_return` and the `comment_carriage_return` fixtures
  under `tests/fixtures/select/` and `tests/fixtures/delete/` (`EDGES.md` row 174, closed).
  The parse-level rows the ledger holds — 175 (a later statement's raw-parse failure in a
  multi-statement `Query`), 176 (a later statement's error position), 185 and 186 (a
  chained `LIKE`, and `ANY`/`ALL` on a `LIKE`-family operator's right) — are recorded
  there.
- **A zero-length delimited identifier is a scanner error, in every position.** `""` is
  not a name anywhere: Postgres refuses it in the scanner, so it is
  `42601 zero-length delimited identifier at or near """"` and not a name error
  (`42602 invalid_name` is the plausible guess and is wrong — measured). pglet's
  `Lexer::scan_quoted_identifier` refuses it, which is the one home. Measured on
  18.6 at the relation name, a column definition, an output alias, a `FROM` alias, a
  column reference, a qualified column, and the `SET`/`SHOW`/`DEALLOCATE` targets — all
  `42601`. A **doubled** quote is a different token and stays legal (`""""` is the
  one-character name `"`), and a zero-length **string** is not an identifier at all:
  `SET search_path TO ''` and `set_config('search_path', '""', false)` are accepted on
  both engines, which is why the `search_path` decider still admits an empty element
  from its string reader. Guards: `lexer::tests`'
  `a_zero_length_delimited_identifier_is_a_scanner_error` and
  `tests/fixtures/commands/zero_length_identifier_*`.
- **Identifier case folding.** pglet applies Postgres's rule: an unquoted identifier is
  ASCII-downcased at the point the parser turns a token into a name, a `"…"`-quoted one
  is exact, and the comparisons after that are plain byte comparisons — **an identifier
  is folded once, where it is read, and compared exactly forever after**. Storing the verbatim spelling and
  reconstructing case-insensitivity at hand-written sites instead produces divergences in
  **both** directions from one cause (measured). Too strict:
  `CREATE TABLE Foo (A INT); SELECT a FROM foo` is `42P01`, and `SELECT 1 AS Bar`
  labels its column `Bar` where Postgres says `bar` (a **silent** wrong — a client
  keying rows by name reads a different key). Too loose: a case-insensitive site cannot
  tell `"PUBLIC"` from `public`, so `DROP SCHEMA "PUBLIC" CASCADE` **drops the real
  `public` schema with every table in it** where 18.6 answers `3F000`, an
  `ON CONFLICT (userid)` arbiter matches an index over `"UserId"` and runs its
  `DO UPDATE`, and `SELECT "NOW"()` answers a timestamp. The fold decides which SQL text reaches a
  persisted name, so it is a store-magic fact (`PGMH` — [storage.md](storage.md) § Store
  versioning).
  `lexer::Token::name` is the fold for an identifier-position token, and the parser's
  name-producing reads go through it — `expect_name`, `expect_col_label`,
  `parse_identifier_primary` and its dotted parts, `parse_table_factor`,
  `expect_type_word`, the PL/pgSQL condition name, the `EXTRACT` field word — so
  `Foo` / `foo` / `FOO` are one name and `"Foo"` is another. The contextual-word
  matchers (`at_ident`/`eat_ident`/`expect_ident`, and `try_function_default`) fold in
  place rather than through it, and match only an unquoted `Identifier` token —
  unlike `parse_copy_header_arg`, which also folds in place but deliberately folds a
  quoted token too, because a `HEADER` argument is a value and not a name (see its
  comment for the measured pairs, which diverge from 18.6 in both directions).
  A **keyword** token folds too, since Postgres's scanner downcases a word before
  classifying it: `CREATE TABLE Sequence` creates `sequence`. A **value grammar** is
  not a name and keeps its own case-insensitivity — `'TRUE'::bool`,
  `INTERVAL '1 YEAR'`, `AT TIME ZONE 'utc'`, `EXTRACT("YEAR" FROM …)` (measured:
  quoted, and still resolving on 18.6), `COPY … (HEADER "ON")` (likewise) — while a
  word Postgres *downcases and then compares* (a function name, an access method, an
  option word, a `COPY` format) is a name and follows the rule. The fold is
  **ASCII-width**, because the oracle folds `A`–`Z` and nothing else — measured, and
  matching the source, where `scansup.c`'s `downcase_identifier` folds a high-bit byte
  (through `tolower`) only when the database encoding is **single-byte** (a property of
  that database's encoding, UTF-8 on the oracle, and pglet models no per-database
  encoding) — and the post-parser sites that
  re-fold a catalog type key (`executor::ddl::execute_create_type`, `execute_drop_type`,
  `executor::type_resolution::resolve_column_type`) use that same width, so none of
  them folds a letter the parser handed through. What that pins, measured with no
  quoting anywhere: `CREATE TYPE CAFÉ AS ENUM ('a'); CREATE TYPE café AS ENUM ('b')`
  registers two types on both engines, and `CREATE TYPE ZÄP AS ENUM ('a');
  CREATE TABLE zt (c zäp)` is `42704` on both — under a Unicode `to_lowercase()` at
  any of the three, the first pair collapses to one type and the second is accepted
  silently (`tests/fixtures/identifier/enum_type_name_*`). The general form is worth
  carrying: a **re-fold is only safe at the width of the fold that produced the name
  it re-folds** — a Unicode fold at a re-fold site is not a stricter version of the
  same rule but a different one. The value grammars keep
  their own width and are not covered by this: `lower()`, `upper()`, `ILIKE` and
  `parse_bool` all fold Unicode here.
  A **name literal** — the `regclass` argument of `nextval`/`currval`/`setval` — folds
  by the same rule inside `SplitIdentifierString`'s grammar (`split_name_literal`), so
  the two name positions always name the same relation: `CREATE SEQUENCE MySeq` and
  `nextval('MYSEQ')` agree, and `nextval('"MySeq"')` does not find it.
  The rule is pinned in both polarities by `tests/fixtures/identifier/*`, plus
  `crates/pglet/tests/case_folding.rs` for the cases a differential fixture cannot
  express (a refusal whose point is what it did *not* destroy, and the spellings where
  pglet's SQLSTATE differs from 18.6's).
  A **value function** needs more than the name: it is matched as a keyword rather
  than a `pg_proc` entry, so `parse_identifier_primary` skips
  `SqlValueFunction::from_name` for a quoted word. That function's arms are the five
  temporal names — `current_date`, `current_timestamp`, `current_time`, `localtime`,
  `localtimestamp` — plus the six session ones: `current_user`, `session_user`,
  `user`, `current_role`, `current_catalog` and `current_schema`. Measured:
  `SELECT "current_timestamp"` is `42703` on both, and dropping the `!quoted` guard
  resolves it here; `SELECT "now"()` names a real function and resolves on both.
  **A following `(` is not one verdict over that set.** Ten of the eleven take no
  argument list and answer `42601` (`current_user()`, `current_date(…)`, the
  `CURRENT_TIMESTAMP(p)` precision form pglet does not model); `current_schema` is
  18.6's one dual-form member — a TYPE_FUNC_NAME_KEYWORD that is *also* `pg_proc`
  1402 — so `current_schema()` falls through to `build_call` and resolves, while
  `current_schema(1)` is that call's `42883`. The split is
  `SqlValueFunction::takes_no_argument_list`, on the enum rather than at the parse
  site, so a member added later decides it where the member is declared.
  The **six** session names that are *not* keywords there stay out of the enum
  deliberately, and pglet agrees with 18.6 on all of them: bare `current_schemas`,
  `current_database`, `version`, `current_setting`, `pg_typeof` and `pg_backend_pid`
  are each `42703` on both engines, because a non-keyword bare word is a column
  reference. Their **called** spellings are ordinary `build_call` arms (`version()`,
  `current_database()`, `current_schema()`, `current_schemas(bool)`,
  `current_setting(name[, missing_ok])`); `pg_typeof` and `pg_backend_pid` are
  unmodelled and stay loud.
  **Residual divergences**, all measured on 18.6:
  - A **quoted type name** does not parse at all (`42601`), where 18.6 reads it as a
    name and answers `42704 type "INT" does not exist` — see `expect_type_word` for why
    admitting the token is not enough on its own.
  - An **ENUM name is ASCII-downcased again where it is stored**, so `CREATE TYPE
    "Loud"` followed by `CREATE TABLE t (m loud)` is accepted here and `42704` there.
    At that width the re-fold reaches only a quoted name — an unquoted one arrives
    from `Token::name` already folded — and it is held there by the line above: the
    reference side cannot spell a quoted type name, so an enum stored under `Loud`
    would be nameable by no column definition and undumpable. The fold rescues a name
    differing from a bare identifier only in ASCII case and leaves the rest —
    `CREATE TYPE "Mood-X"`, the unquoted `CREATE TYPE Sequence`, and the unquoted
    `CREATE TYPE MÖÖD` (stored `mÖÖd`) are each accepted by both engines and each
    make `pglet_server dump` exit non-zero having emitted nothing
    (`EDGES.md`).
  - A **quoted lower-case function name** that pglet models as a function but 18.6
    holds only as grammar resolves here where 18.6 answers `42883`: measured for
    `"coalesce"(1,2)`, `"nullif"(1,2)`, `"greatest"(1,2)`, `"least"(1,2)` and
    `"trim"(' x ')`. The spellings backed by a real `pg_proc` entry agree —
    `"substring"('abcdef',2,3)`, `"count"(*)`, `"now"()`, `"upper"('a')`,
    `"lower"('A')`, `"gen_random_uuid"()` — and so do the upper-case-quoted spellings
    measured (`"COALESCE"(1,2)`, `"TRIM"(' x ')`), which are loud on both. Closing it
    means recording in `build_call`'s table which names are `pg_proc` entries and
    which are special forms. A **set-quantifier** spelling of any of those five is loud
    on both sides rather than silent, and points the other way: `planner`'s
    `grammar_only_name` refuses it `42601` — the code 18.6 gives the *bare* spelling —
    against 18.6's `42883` for the quoted one. Measured: `"coalesce"(DISTINCT 1,2)`,
    `"trim"(ALL ' x ')` and `"trim"(DISTINCT ' x ')`. That is why `trim` is in that set
    and `substring` is not: `pg_proc` holds `substring` and not `trim`, so the quoted
    `substring` spellings resolve on 18.6 and pglet's ordinary verdicts already agree
    (`"substring"(ALL 'abcdef',2)` answers `bcdef` on both,
    `"substring"(DISTINCT 'abcdef',2)` is `42809` on both) — and without `trim` in that
    set, `"trim"(ALL ' x ')` would answer `x` here, a **silent** member of this gap rather
    than a loud one.
  - A quoted `"EXCLUDED".c` is `42P01`, as on Postgres. The deciding site is
    `EvalEnv::resolve_outer_slot` rather than the planner scope's qualifier test, and
    that chain declines a qualifier no frame owns rather than falling to a bare match,
    which would let any unknown qualifier in that action read the `excluded` row's
    column of that name.
  - **No `NAMEDATALEN` truncation** — self-consistent within pglet, so both name
    positions still agree *here*. Self-consistency is not agreement with 18.6, and the
    sharper reading is the **No 63-byte (`NAMEDATALEN`) identifier truncation** entry
    below.
  The *unquoted* `3F000`-vs-`42P01` split for a foreign schema qualifier is a separate,
  live divergence needing a statement context rather than anything about quoting: the
  DDL forms measured (`CREATE TABLE noschema.qq`, `DROP TABLE`,
  `CREATE INDEX … ON noschema.t`, `ALTER TABLE`, `CREATE SEQUENCE`, `TRUNCATE`) are
  `3F000` on 18.6 and `42P01` here, while the DML lookups are `42P01` on both. See
  `EDGES.md`, which carries the record and the forms that
  answer a third way.
- **A typed constant is a cast, and an identifier followed by a string literal is
  always one — read at a *different type-name production*.** `gram.y`'s `AexprConst`
  gives four productions —
  `ConstTypename Sconst`, `func_name Sconst`,
  `func_name '(' typmod ')' Sconst`, and `ConstInterval Sconst opt_interval` —
  and pglet lowers the first three through the node `'…'::type` lowers to
  (`Parser::try_typed_constant` → `build_cast`), so the value, the declared type OID,
  the input-conversion SQLSTATE, the enforcement of a **spelled** typmod, the unaliased
  output column name and the deparse are the cast path's rather than a second
  implementation's.
  18.6's own `pg_get_constraintdef` renders a stored `CHECK (d > DATE '…')` as
  `(d > '…'::date)`, which is what that shared lowering already emits.
  **`ConstTypename` is not `Typename` narrowed by one rule** — it admits neither
  `GenericType` nor either `ConstInterval` arm, and `Typename`'s array bounds and
  `SETOF` are outside it — and **the part pglet carries** is the fixed-length default,
  `ConstCharacter`'s reason to exist in `gram.y`'s own words: "to allow defaulting
  fixed-length types such as CHAR() and BIT() to an unspecified length … this makes no
  sense for constructs like CHAR 'hi'".
  `CharacterWithoutLength` installs `bpchar(1)`; `ConstCharacter` clears it again.
  pglet splits at the resolver rather than in the type-name reader — one
  `executor::type_resolution::resolve_type_name` under two entries, the constant one
  skipping that default and nothing else — so `char 'ab'` and `character 'ab'` are `ab`
  while `'ab'::char` and `CAST('ab' AS char)` are `a`, and a
  spelled `char(1) 'ab'` is `a` again; each measured on both engines. A `char`
  **column** is *not* a fourth member of that list: `'ab'` into one is `22001` on both
  engines (18.6 naming `character(1)`, pglet `character varying(1)`), assignment to
  `bpchar(n)` being no explicit cast.
  **Parenless is the predicate, not "no length survived the read".** `ConstCharacter`
  clears the `CharacterWithoutLength` arm and passes `CharacterWithLength` straight to
  `typenameTypeMod`, so a length the *type* refuses — below 1, or above 10 485 760 — is a
  spelled modifier at *both* positions and collects the default at neither: `char(0) 'ab'`
  and `'ab'::char(0)` are both `22023 length for type char must be at least 1`, on both
  engines, and `CREATE TABLE q (c CHAR(0))` builds no table rather than a `char(1)` or an
  unconstrained column. The two other keyings are wrong in opposite directions — the
  decline read as "no modifier was spelled" collects the default and caps at 1 a length
  nobody wrote, and read as "no cap" it builds the column that stores `'abcdef'` — and the
  refusal is what removes the third state both readings need
  (`docs/coercion.md` § A modifier the type refuses,
  `EDGES.md` row 124). Pinned differentially by the
  `types/refused_typmod_*` fixtures at both positions, and end-to-end — values, and the
  catalog a refusal must not leave — by `crates/pglet/tests/typmod_decomposition.rs`. (The
  **writes** in that file are the *polarity* half's, over modifiers the refusal must not
  reach; a refusal has no write to test.) The `varchar`
  branch of the same arm spells no default at either position and cannot see the split.
  **A number the window refuses parts the two positions**, and the mechanism is the
  speculative restore rather than anything about `char`: `commits_on_open_paren` is `float`
  alone, so at every other carrier lead word a `(…)` that fails the reader's own window
  restores the speculation and the window falls to the function-call path. What answers then
  is the restored cursor's — `0A000 function not supported: <word>` where the lead word is
  the whole spelling, a `42601` of `SELECT`'s own grammar where a tail is left over
  (`character varying`, the one spelling whose SQLSTATE agrees). So `char(3.5) 'ab'`,
  `char(2147483648) 'ab'`, `char(99999999999999999999) 'ab'` and their `character` /
  `varchar` / `character varying` forms are that `0A000` / `42601` against 18.6's `42601`;
  their **cast** spellings agree at `42601`. `char(-1) 'ab'`, `char(5,2) 'ab'`,
  `numeric(+5) '1'` and the `timestamp(3.5) '…'` / `time(3.5) '…'` share are members too,
  while the three number shapes above are not members at `numeric` — they agree there.
  `EDGES.md` rows 116 and 124 carry the derived
  membership; the `0A000` at a *modifier-bearing* uncommitted lead word is not the only
  thing the constant position gets wrong here.
  Three of the five spellings 18.6's `character:` production reaches `bpchar` under are
  unmodelled here and so inert: `nchar` is `42704` at both positions, and
  `national char` / `national character` is `42704` at a declared one and `42601` at a
  constant one (the multiword lead is outside the speculation set — below). `ConstBit`'s
  share is inert the same way — pglet models neither `bit` nor `varbit`, so `bit '101'`
  is `42704` before a default could be suppressed — as is `timetz`'s share of the
  temporal-precision rule. The constant position is a member of `EDGES.md` row 95: it
  declares 1043 against 1042, `column_default` keeps a bare
  `character varying`-against-`bpchar` name gap where the other three catalog renderings
  differ by a whole extra coercion node, and 18.6's blank-insensitive `bpchar` is
  reachable through a constant that carries trailing blanks. Row 95
  holds the measured cells.
  Two **dispatch shapes**, because only one is decidable from the two-token window:
  an identifier immediately followed by a string is a typed constant unconditionally
  (18.6 answers `SELECT foo 'x'` with `42704 type "foo" does not exist` even when
  `foo` is a real column, so the commit is safe and is what keeps a bare `date`
  resolving as a column while `date 'x'` does not); a **multi-word or
  modifier-bearing** type name (`DOUBLE PRECISION '…'`, `CHARACTER VARYING '…'`,
  `TIMESTAMP WITH TIME ZONE '…'`, `NUMERIC(5,2) '…'`) is not, and every one of its
  lead words is a legal bare column name on both engines, so it parses speculatively
  from a `TokenStream::checkpoint` and restores the cursor when no string follows —
  unless the window is one it has **committed** to, below.
  The type name is the **scalar** production only: `int4[] '{1,2}'` is `42601` on
  18.6 (the array spelling is `_int4 '{1,2}'`, which pglet answers `42704`).
  Residuals, all measured: a **quoted** type name still does not parse
  (`"date" '…'` is `42601` here against a value there — the same
  `expect_type_word` gap the bullet above records for `::"int4"`); a
  **schema-qualified** one likewise (`pg_catalog.date '…'`); and the conversion is
  folded during the fused parse+plan, so `SELECT DATE 'x' 'y'` reports the literal's
  `22007` where 18.6 reports the trailing token's `42601` (a phase-order divergence
  the same shape `SELECT 1/0 'y'` has, not one this grammar introduces — and the same
  one a boolean gate's verdict reaches ahead of the end-of-input check, `EDGES.md`
  row 128). The
  speculative path separates its two phases for a reason worth keeping: reading the
  type name is the *question*, resolving it is an *answer*, and running both under
  one `if let Ok(…)` would swallow the second — `SELECT time with time zone '10:00'`
  would report `42601` where the single-word `timetz '10:00'` reports the `TIMETZ`
  rule's `42704`, one spelling of one type answering two codes. **And the speculation
  has a third outcome — a window the reader commits to.** Where a lead word followed by
  `(` can begin nothing but a type name, a failure inside the speculation is the
  statement's answer and the cursor is not put back. `parser::expr`'s
  `commits_on_open_paren` is the membership test and `float` is what it admits, because
  `opt_float` refuses `FLOAT(0)` and `FLOAT(54)` *in the grammar* on 18.6 and a restore
  would swallow that `22023`, demoting it
  to the function-call path's `0A000 function not supported: float` — the same demotion
  the two-phase split records. Committed, `SELECT float(0)` is the `22023`, and
  `float(1)`, `float(1) + 1`, `float(1.5)`, `float(1,2)` and `float(2147483648)` are
  `42601`, each agreeing with 18.6. **What decides the window is the `(`, not the lead
  word**: bare `float` is an ordinary column reference on both engines
  (`42703 column "float" does not exist`) and still restores. 18.6 splits the same way
  and for the same reason — `float` is a `COL_NAME_KEYWORD`, excluded from `func_name`,
  so `float (` can begin nothing but `Numeric` — and the membership here is deliberately
  narrower than that reason reaches: the lead words that pass the same `kwlist.h` test
  and stay uncommitted, and the ones that fail it and must never join, are `EDGES.md`
  row 116's to enumerate. `NATIONAL CHARACTER [VARYING]` is the lead word the reader
  still cannot follow and is therefore *not* in the speculation set:
  `SELECT national character '5'` is a value on 18.6 and `42601` here.
- **A `'(' Iconst ')'` modifier is read by one reader, and what the number *means* is
  the type's.** `gram.y` spells these pglet type-modifier windows with a single
  `Iconst` — `ConstDatetime`'s `(p)`, `SimpleTypename`'s `ConstInterval '(' Iconst ')'`,
  `CharacterWithLength`'s `character '(' Iconst ')'`, and `opt_float`'s `(n)` — so
  `Parser::opt_precision` serves each of them, and an `ICONST` being an unsigned integer that
  fits `int32` is what makes `timestamp(3,1)`, `timestamp(3.5)`, `timestamp(-1)`,
  `timestamp(2147483648)` and the same four spellings of `char(n)` / `varchar(n)` each
  `42601` on both engines. Expecting `)` after one number covers the comma; the `i32`
  parse covers the rest. Reading the **character length** through a weaker window instead
  would let `'abcd'::char(3.5)` resolve to an unconstrained `varchar` and answer `abcd`,
  or — with the type refusing an out-of-window length — answer that refusal's `22023`
  about a number the grammar never admits. What the number then
  means is decided by the type and not here: a temporal `(p)` is clamped to `6`, a
  character length is refused outside `1 ..= 10485760`, and `FLOAT(n)` spends it selecting
  `real` or `double precision` — the last being the one this reader's caller decides,
  because `opt_float` decides it in the grammar (`docs/coercion.md`). `numeric(p[,s])` is
  deliberately **not** one of them — its modifier is an `expr_list` read by
  `passthrough_type` and coerced to `int4` at the resolver ([coercion.md](coercion.md)
  § A modifier the type refuses).
- **An interval qualifier sets a bare number's unit, then masks the result — and it
  is the *number of unlabelled fields*, not the range-vs-single-field distinction,
  that decides whether the qualifier is safe to apply.**
  `ConstInterval Sconst opt_interval`'s trailing field word is part of the literal,
  not an output alias: read as one, `INTERVAL '90' MINUTE` answers ninety *seconds* —
  a wrong value, not a parse wall. `Parser::parse_interval_qualifier` reads the six
  single-field spellings; a literal that is a bare signed decimal is **re-decoded**
  with the qualifier's unit appended (`interval_from_unit` hands `90 minute` to the
  one interval decoder rather than doing the arithmetic a second time), and the
  result is then masked to that field (`AdjustIntervalForTypmod`: fields right of
  the named one zeroed, those left of it kept, so `'1 day' MINUTE` is still
  `1 day`). `WEEK` is deliberately absent, because it is absent from `opt_interval`
  too.

  **What a qualifier changes is the reading of every *unlabelled* field, and that is
  not a property of the `X TO Y` forms.** `DecodeInterval`'s `DTK_NUMBER` case
  assigns an unlabelled number the field the range names and then advances to the
  next coarser one, so a run of them decodes positionally: `INTERVAL '1 2' HOUR` is
  `1 day 02:00:00` on 18.6 under a **single** field word. pglet models no positional
  decode, and the shapes that would need it are refused rather than answered — a
  second unlabelled field is `22007` in the decoder itself (which is 18.6's answer
  for every such literal but `'1 2' HOUR`, and closes the same wrong value in the
  *unqualified* `INTERVAL '1 2'`), and the `X TO Y` ranges are `0A000`.
  `INTERVAL(p) '…'` and a `SECOND(p)` precision are likewise unparsed and loud.
  Residual: `INTERVAL '1 2' HOUR` is `22007` here against a value there.
- **`x IS [NOT] TRUE / FALSE / UNKNOWN`** is Postgres's `BooleanTest`, its own IR
  node (`Expr::BooleanTest`) rather than a lowering to `IS [NOT] DISTINCT FROM` —
  18.6 renders a stored `CHECK (b IS TRUE)` back as `CHECK ((b IS TRUE))`, and the
  operand gate's message names the spelling. `UNKNOWN` is not in the keyword table,
  so it stays a usable column name; `IS` binds tighter than `NOT`/`AND` and looser
  than a comparison, so `a = b IS TRUE` groups as `(a = b) IS TRUE`.
- **Adjacent string-literal concatenation** across a newline (`'a'\n'b'` → `'ab'`, the
  SQL-standard rule) isn't done; fails loud, but where the general `SET` value reader passes it,
  which drops the value (`EDGES.md` row 256).
- **No 63-byte (`NAMEDATALEN`) identifier truncation** — pglet accepts arbitrarily long
  identifiers where 18.6 truncates each to `NAMEDATALEN - 1` = 63 bytes as the parser
  reads it, under a `42622` **notice** rather than in silence. What it costs inside pglet
  is not the long name it stores; it is that a name 18.6 resolves is `42703` here (a 64-byte
  reference to a 63-byte column), and that two names differing only past byte 63 are
  **one** column there and two here — so any rule that reads a name list as a *set* reads
  a different set on each engine. Outside pglet it costs the dump: two declared relation,
  column or sequence names that share their first 63 bytes are two objects here and one
  name on 18.6, so the script restores short there with `psql` exiting `0` — measured for
  two tables with rows, two sequences and a two-column table (`EDGES.md` row 278's residual
  holds the constraint-name member). Measured over `tt (<63 × 'c'> INT, x INT)` with two such
  names: the `INSERT` column list is `42701` on 18.6, the `UPDATE` `SET` list `42601` and
  `COPY` `42701`, all three `42703` here ([names.md](names.md) § What the refusals do not
  carry). It also decides the **size of an error message**, and read for the whole
  *statement* the wider side is 18.6's: its `column "…" does not exist` carries the
  truncated 63 bytes where pglet's carries the whole name — 163 B against 200 053 B for a
  200 000-character one, counting the whole `ErrorResponse` packet as a raw v3 client
  reads it — but the truncation is a notice rather than silence, and that
  `42622 identifier "…" will be truncated to "…"` **`NoticeResponse`** inlines the
  untruncated name at 200 167 B, so the statement is **200 330 B there against 200 053 B here**.
  Both halves are quoted and unquoted alike, and every identifier position measured does
  the same: a relation name is 163 + 200 167 B against 200 055 B, and an unimplemented
  function name 265 + 200 167 B against 200 070 B. **What this
  class costs is the divergence, not the width** — a message that names a *value* is
  wide on both engines, `SELECT '<200 000 digits>x'::numeric` being 200 102 B on 18.6
  against 200 071 B here. Beside the identifier the two other spellings that could carry
  their own length do not: an operator name is bounded at `NAMEDATALEN` by the scanner's
  own rule (`lexer::core`'s `scan_op_chars`), and a numeric literal's refusal names none
  of it (`SELECT 1e<200 000 nines>` is 59 B here, 97 B there).


## A function-call set quantifier is 18.6's, both keywords

`func_application`'s `func_name '(' DISTINCT func_arg_list opt_sort_clause ')'`
production sets `agg_distinct`, and `ParseFuncOrColumn` refuses it for anything
that resolves to an ordinary function — the `if (agg_distinct)` arm, the next
`if` below the `agg_star` arm the niladic builtins' wildcard refusal
implements. Dropping the modifier at every arm but the aggregates is **silent**
throughout — a plausible value, and no error channel anywhere — and at the
persisted-expression sites it stores the call with the modifier stripped, so
`column_default` and the dump render something the user never wrote. The
measured members of that class and its residuals are `EDGES.md` row 51's; what
follows is the rule.

**The class is three rules, not one.** (1) `DISTINCT` over a non-aggregate is
`42809`. (2) `gram.y` bounds *either* keyword: both quantifier productions
require a **non-empty** `func_arg_list`, and the `func_name '(' '*' ')'`
production combines with neither — so the whole cross-product {`DISTINCT`,
`ALL`} × {`)`, `*`} × {ordinary call, window call} is `42601` there, measured
cell by cell. (3) `ALL` is the no-op quantifier — 18.6 accepts it on any call
that resolves, aggregate or not (`gram.y`'s comment on that production says it
would like to mark the node "must be an aggregate" and has nowhere to put the
bit), where refusing every spelling `42601` is an over-rejection. The three
move together: **dropping `ALL` in the parser would create a new silent
over-acceptance**, because the names 18.6 owns in its own grammar refuse
either keyword, so `coalesce(ALL 1, 2)` — `42601` there — would answer
`1`. So the quantifier is carried as *which* keyword (`planner::SetQuantifier`),
and `grammar_only_name` refuses both over the names 18.6's `pg_proc` **lacks**
whose call would otherwise *answer* — `coalesce`, `greatest`, `least`,
`nullif`, and `trim`, whose bare spelling has its own parser route and whose
*quoted* one does not. The set is derived per member from a catalog sweep of
every name the dispatch matches, not from 18.6's grammar list: `substring` is on
that list and is a `pg_proc` entry, so its quoted spellings already agree and it
must stay out, while `uuid_generate_v4` is absent from `pg_proc` and stays out
too, being niladic — its arity check already refuses every quantified spelling
at 18.6's own code. The rule adds no persisted shape, so it is no store-magic
fact — `SetQuantifier` never reaches an `Expr`, and `Expr::Aggregate`'s own `distinct` field is untouched.

**All three rules reach the FROM clause too**, where `unnest(…)` is the one
function call pglet models (`parse_table_factor` dispatches a derived table,
that `unnest`, a CTE reference, a catalog view and a base table, and nothing
else). There the two keywords part company by a *second* mechanism:
`transformRangeFunction` expands a multi-argument FROM-clause `unnest` into one
call per argument only for an **undecorated** call (`!fc->agg_distinct` among
its guards), so `ALL` expands and answers while `DISTINCT` leaves an ordinary
`unnest(…)` for `ParseFuncOrColumn` — one array resolves and is `42809`, two
resolve to no signature and are
`42883 function unnest(integer[], integer[]) does not exist`, ahead of the
quantifier. The argument transform stands above **both** of those verdicts here
as it does at `build_call` — 18.6 transforms the arrays before
`transformRangeFunction` reads the decoration at all — so
`FROM unnest(DISTINCT ARRAY['a'::int]) AS u(x)` is `22P02` there and so is its
two-array spelling, where the arity refusal would otherwise answer `42883`;
hence `refuse_from_unnest_quantifier` takes the array *expressions* rather than
their count.

**And a third route reads one — the DDL `DEFAULT` classifier.** `parser::ddl`'s
`try_function_default` recognizes a bare `DEFAULT nextval('s')` by tokens and
parses that call's argument list itself, so nothing written there reaches
`parse_function_call`; `nextval` is the only arm whose call takes an argument
and so the only member, the niladic arms' own `expect(RParen)` already being the
grammar bound (`DEFAULT now(ALL)`, `now(DISTINCT)` and `gen_random_uuid(ALL)`
are `42601` on both). **Its two positions are the general path's**, each a
measured pair rather than a symmetry: the verdict runs *after* the closing
paren, so `DEFAULT nextval(DISTINCT 's' junk)` is `42601` on both where raising
it above the `expect` answers `42809`; and *before* the name-literal reduction,
where `sequence_call` puts it, so `DEFAULT nextval(DISTINCT 'bad name')` is
`42809` on both where reducing the literal as it is read answers pglet's
`42602`. And that is `parse_column_default`'s invariant that the bare and the
parenthesized spelling are one stored fact: `column_default` reads
`nextval('s'::regclass)` for bare-`ALL`, parenthesized-`ALL` and unquantified
alike, on both engines.

**The deferred name-literal reduction decides an unquantified class too**, by
the classifier rather than by the quantifier. Reducing the literal *after* the
closing paren puts pglet's `42602` behind the syntax error 18.6 reports, so an
unquantified spelling whose bad token stands **inside** the call — where the `)`
should be, or in a cast type after the literal — is `42601` on both engines, as
its parenthesized spelling is:
`DEFAULT nextval('bad name' junk)`, `nextval('bad name'::)` and
`nextval('bad name'::123)` at each of `CREATE TABLE`, `ADD COLUMN` and
`ALTER COLUMN … SET DEFAULT`, plus the unterminated `nextval('bad name'` at the
latter two, each measured on 18.6 and on both pglet builds. The condition is
*inside*, and a bad token **outside** the call is the control that says so:
`CREATE TABLE t (a BIGINT DEFAULT nextval('bad name');`, whose unterminated
element is the table's own column list, is `42602` on both pglet builds against
18.6's `42601` — a member of the later-syntax-error class `EDGES.md` row 51 records
rather than of this one. The well-formed `DEFAULT nextval('bad name')` is `42602` on
**both engines**, which is what identifies the reduction's *position*, and not the
reduction, as what decides the class.
`ddl/nextval_column_default_name_literal_yields_to_the_paren_loud.sql` pins a
member of it.

**The verdict's position is measured at three scales.** The grammar bound is the
parser's, stated once there and not restated in the dispatch. The grammar-name
refusal runs **ahead** of the arity checks, because 18.6's is a parse error that
outranks everything: `nullif(DISTINCT 1)`, wrong on both counts, is `42601`
on both. The `42809` runs **after** every check standing in
for `func_get_detail`'s signature resolution and **before** everything standing
in for something 18.6 raises later — `upper(DISTINCT 'a', 'b')` is `42883` on
both, `now(DISTINCT 1)` is `42883` on both — which is also why the **six niladic
builtins contribute no member of the silent half**: `niladic_call` refuses the
argument list before anything reads the quantifier, so the non-empty list
decides it rather than the modifier. `jsonb_build_object(DISTINCT 'a')` and
`nextval(DISTINCT 'bad name')` are `42809` there against pglet's own
`42804`/`42602`, which is why those two arms carry the verdict inside the
dispatch rather than at its tail.

**Above the signature resolution sits a scale of its own, and it is bounded by
the quantifier at both ends — written on one arm it splits the names, and written
for every call it pre-empts the DDL verdicts (below).** `transformFuncCall` transforms
every argument (`transformExprRecurse`) before `ParseFuncOrColumn` sees the
call, so a **constant cast** — coerced during that transform — outranks the
signature resolution and the modifier alike, for every name.
`SELECT upper(DISTINCT 'a'::int)` is `22P02` on both engines, as are the
`abs` / `concat` / `length` / `to_char` / `date_part` / `starts_with` / `unnest` /
`jsonb_build_object` spellings and both spellings of the same call in a column
`DEFAULT`; written on the sequence arm alone, the transform would agree for the
three sequence names and answer `42809` for every other. The one measured member
whose answer is not `22P02` is `set_config(DISTINCT 'x'::int, …)`, which answers
`y` here — its discarded argument makes it silent (`EDGES.md` row 51). The transform is one call
above the dispatch (`planner::transform_call_args`) and one at the FROM route,
and its position is fixed by a measured pair at each end: **below** the
grammar-only refusal, since `coalesce(DISTINCT 'a'::int, 1)` is `42601` there
where the unquantified `coalesce('a'::int, 1)` is `22P02`; **above** every arity
check, since `setval(DISTINCT 'sq'::int, 1, 2, 3)` — wrong on the count too — is
`22P02` there where `setval(DISTINCT 'sq', 1, 2, 3)` is `42883` on both.

**A typmod'd cast is a constant cast and its modifier is not**, which is one
node kind and one non-obvious half. `'abc'::numeric(5,2)` is the same
`stringTypeDatum` call as `'abc'::numeric` — `22P02` ahead of the quantifier —
while the modifier itself is applied by a *separate* `coerce_type_typmod` node
the **planner** folds (`coerce_type` passes `inputTypeMod = -1` for every type
but `interval`: "Any length constraint will be applied later by our caller").
The pair that measures it: `concat(DISTINCT 'abc'::numeric(5,2))` is `22P02` on
18.6 and `concat(DISTINCT '12345'::numeric(3,1))` is `42809` — the quantifier
beating the overflow the unquantified spelling does raise (`22003`, on both
engines). pglet builds a distinct `Expr::CastTypmod` for the modifier'd
spelling, so the transform matches that node too, and must not apply the
modifier. What the arm covers, named: `concat(DISTINCT 'abc'::numeric(5,2))`
and its `upper` twin, the two-argument
`upper(DISTINCT 'abc'::numeric(5,2), 'b')` against the arity, and the
`set_config`, column-`DEFAULT`, FROM-`unnest` and `nextval` positions — `42809`
here at every one of them but the arity's `42883`, against `22P02` there. The
truncating `'abcdef'::varchar(2)` still answers `AB` through `upper(ALL …)` on
both. `interval(p)` reaches the arm, because pglet keeps that precision —
`'x'::interval(3)` builds a `CastTypmod` — so 18.6's own `interval` carve-out (the one
type whose `inputTypeMod` is passed through) is in this arm's reach. The modifier is
still not applied here, which two measured spellings do not separate:
`concat(DISTINCT 'xyz'::interval(3))` is `22007` and
`concat(DISTINCT '0.5 sec'::interval(0))` is `42809`, each on both engines.

**The transform runs only for a call carrying a quantifier, and that gate is a
correctness rule rather than a scoping convenience.** pglet raises this whole
family in its *parser*, ahead of every executor-time statement gate, where 18.6
cooks a column `DEFAULT` and a `CHECK` in `AddRelationNewConstraints`, *after*
`heap_create_with_catalog`. Run for an unquantified call it would therefore
pre-empt the DDL verdicts it stands in front of — measured on an ungated build:
statements whose `42704` / `42P07` / `42P16` / `42703` / `42P01` / `42601` become
`22P02`, at a column `DEFAULT` and at a `CHECK`, through `CREATE TABLE`,
`ALTER TABLE … ADD COLUMN`, `ALTER COLUMN … SET DEFAULT`, `ADD CONSTRAINT` and
`CREATE INDEX`; `IF NOT EXISTS` skips (`upper`, `to_char`, `substring`, `trim`, and
a `CHECK`'s `upper`); and, in the other direction, the within-expression source
order, which inverts rather than closes: 18.6 cooks a stored expression left to
right, so `CHECK (upper('a'::int) = 'A' AND nosuchcol > 0)` is `22P02` there and
its mirror is `42703`, while pglet answers `42703` to **both** spellings (its own
walk raises the column-binding error and never the cast) and the ungated hoist
`22P02` to both. One spelling agrees either way and the trade is symmetric, which
is why the closing site is the silent member's own (`EDGES.md` row 51) rather than
this position. The worst of them is a **success**:
`CREATE TABLE IF NOT EXISTS <existing> (b TEXT DEFAULT upper('a'::int))` is a skip
on 18.6 and here, and `22P02` under the ungated hoist, with a client's `prepare()`
failing at `Parse`. The invariant the gate keeps is that **no unquantified
statement this transform can reach changes phase or answer**: every ordinary
expression position is `22P02` on 18.6 and here, measured (`upper`, `abs`,
`length`, `concat`, `to_char`, `date_part`, `starts_with`, `count`,
`jsonb_build_object`, `nextval`, `setval`, `unnest`, `trim`, `substring`, in a
select list and in `INSERT` / `UPDATE` / `DELETE`), because the plan-time
`planner::validate_const_casts` pass already answers there — except the two
sequence names, whose name argument never reaches that pass and which
`planner::sequence_call` answers with a check of its own. One check the gate
does *not* stand in for is that sequence arm's: `sequence_call` reduces its name
argument to a `String` and discards the expression, so a constant cast there is
raised by that arm or not at all, and deleting it as redundant of an *ungated*
transform would bring back a **write** — `nextval('sq'::int)` and
`setval('sq'::int, 50)` answering a value and moving the counter against 18.6's
`22P02` (`crates/pglet/tests/set_quantifier_verdicts.rs`'s
`an_unquantified_sequence_name_cast_does_not_move_the_counter`, red on that
arm's revert). Both unquantified polarities of the gate itself are pinned by
`ddl/if_not_exists_skip_beats_a_const_cast.sql` and
`ddl/column_type_beats_a_const_cast_loud.sql`.

A third face of the same node is loud and pglet-only: the plan-time
`planner::validate_const_casts` pass matches `Expr::Cast` and not
`Expr::CastTypmod`, so `SELECT 'abc'::numeric(5,2)` raises its `22P02` at eval
where 18.6 raises it in parse analysis. Widening that pass is **not** the
repair: it runs ahead of the range table here, so `SELECT 'abc'::numeric(5,2) FROM
nosuchrel` — `42P01` on both engines — becomes `22P02`, measured on a build with the
kind added, on which **the whole fixture corpus is green**. So the argument
transform takes the kind and the plan-time pass does not
(`planner::validate_arg_const_casts` against
`planner::validate_const_casts`), and that split is held by a probe rather than
by a gate.

**Three siblings of the same `ParseFuncOrColumn` block stay out**, loud here
already and for a different reason: `upper('a' ORDER BY 1)` is 18.6's
`42809 ORDER BY specified, …`, `upper('a') FILTER (WHERE true)` its
`42809 FILTER specified, …`, and `upper('a') WITHIN GROUP (ORDER BY 1)` its
`42883` — the sort argument joins the signature first, so only a name that still
resolves reaches that arm (`concat('a') WITHIN GROUP (ORDER BY 'b')` is
`42809 WITHIN GROUP specified, …`). All `42601` here; pglet's grammar has none
of the three productions, which is also why none is fixturable — an
`@expect-error` compares the codes. The `ORDER BY` sibling has a **FROM-route**
member too: `FROM unnest(ARRAY[1,2] ORDER BY 1)` is
`42809 ORDER BY specified, but unnest is not an aggregate function` there
against `42601` here, and its `DISTINCT` spelling
`FROM unnest(DISTINCT ARRAY[1,2] ORDER BY 1)` is 18.6's `42809` for the
*quantifier* — the modifier is judged before the sort clause — against the same
`42601` here. `EDGES.md` row 51 carries them as divergences; the reason they are
not this rule's to close is the missing productions.

**What pins the rule.** The `set_quantifier_*` differential fixtures —
`functions/` covering one member per `build_call` arm family (math, string,
binary, variadic, jsonb, sequence, single-argument, `set_config`, `Extract`),
the quoted and schema-qualified spellings, the grammar names in both keywords,
the FROM-clause `unnest` route in both keywords over both array counts plus its
bound and the position of its verdict against the call's closing paren, the
grammar bound (six of its eight cells; the two window-`*` cells are measured
`42601` on both and reach the identical parser call), the precedence pairs and
both accept polarities — the argument transform's own, which are the pairs above
rather than one statement: the headline (`upper(DISTINCT 'a'::int)`), a
**later** argument's cast on a non-string arm (`concat(DISTINCT 'a', 'b'::int)`,
the one a first-argument-only check passes), the transform's two bounds
(`coalesce(DISTINCT 'a'::int, 1)` above it, `setval(DISTINCT 'sq'::int, 1, 2, 3)`
below it) and the FROM route's two array counts — plus the `sequences/`, `ddl/`,
`select/`, `insert/`, `update/` and `delete/` positions. The `ddl/` set is two
things: it covers the four
persisted-expression sites through the **general** expression parser (a column
`DEFAULT`, a CHECK, an index key, an index predicate, each spelled `upper`), and
separately the column-`DEFAULT` site's **second** parser, the classifier,
through `nextval` — the `ALL` accept over the `column_default` its three
spellings share, the `CREATE TABLE` and `ALTER COLUMN … SET DEFAULT` refusals
(two fixtures because the `b_expr`/`a_expr` grammar modes genuinely split there,
where `ADD COLUMN` is the same call site under the same mode as `CREATE TABLE`
and is none), and the verdict's two positions, the second with an unquantified
control beside it. And `crates/pglet/tests/set_quantifier_verdicts.rs` for what
a fixture cannot hold: the counter and the row set after a refusal, the message
text that separates this refusal from its `<name>(*)` sibling in the same error
variant — at the classifier route as well as the general one — and the quoted
`"trim"` member, whose refusal both engines make at different codes. The two
gate polarities are `ddl/if_not_exists_skip_beats_a_const_cast` and
`ddl/column_type_beats_a_const_cast_loud`, both red on the ungated hoist.

## A string literal is assembled in 18.6's units, and decoded by the grammar its scanner matches

`lexer::escapes` is the single home: `decode_escape` for `E'…'` and `decode_unicode`
for `U&'…'`, one call site each in `lexer::core`'s `scan_string`. A plain `'…'` reaches
neither — under `standard_conforming_strings` it collapses `''` and reads no backslash
at all, and a `"…"` identifier collapses `""` and reads none either.

**The unit is a byte.** 18.6's `<xe>{xeoctesc}` / `<xe>{xehexesc}` do
`unsigned char c = strtoul(…); addlitchar(c)`, so a `\xHH` / `\ooo` escape contributes one
**byte**, truncated to eight bits, and everything else contributes a character's UTF-8.
The finished literal is then validated **once, left to right**, at the closing quote
(`pg_verifymbstr`). The unit is what the answer turns on: `E'\xC3\xA9'` is one character,
`= 'é'` being `t`, and a lone `E'\xE9'` is `22021` rather than a one-character string. A
reading in **code points** gives neither — two characters for the first and a value for
the second. The truncation is the same unit seen at its other end: `E'\777'` is the byte
`0xFF` and `E'\400'` a NUL, both `22021`, where an untruncated read names the characters
U+01FF and U+0100.

Rust's own UTF-8 acceptance set is `pg_utf8_islegal`'s byte for byte — overlongs,
surrogate bytes and values above `U+10FFFF` refused by both — so the scan
(`pglet_core::encoding`'s, whose other doors are `docs/encoding.md`'s) is
`str::from_utf8` plus a separate look for the NUL, which Rust takes and
`pg_utf8_verifystr` does not. The failure is at the **first** of the two, which is what
makes `E'\xE9\x00'` report `0xe9 0x00` rather than the NUL alone, and the message is
`report_invalid_encoding`'s: the bytes from that offset, `min(pg_utf_mblen(lead),
remaining)` of them. 18.6 caps that count at 8 as well, which `pg_utf_mblen` — answering
1, 2, 3 or 4, its 5- and 6-byte arms being `#ifdef NOT_USED` — cannot reach, so the cap
has no counterpart here.

**An escape the grammar does not match is a character, not an error.** `{xehexesc}`
needs one or two hex digits; a `\x` that fails it is matched by the shorter `{xeescape}`
instead and `unescape_single_char('x')` returns the letter, so `E'\x'` is `x` and
`E'\xZZ'` is `xZZ`. `unescape_single_char`'s arms are `b f n r t v`, the `v` being a
vertical tab and not the letter, and its default returns the character — which is what
makes `E'\z'`, `E'\8'` and `E'\9'` the bare characters `z`, `8` and `9` (`{xeoctesc}`
claims only `[0-7]`, so the digits never reach it).

**Two adjacent Unicode escapes spelling a UTF-16 surrogate pair are one character.** The
rule is one function (`pair_surrogate`) both string-literal decoders call; what differs is what counts
as *adjacent*, because 18.6 spells it as a scanner state in one and a saved variable in
the other.

In an `E'…'` a first half puts the scanner in the exclusive `xeu` state, whose whole rule
set reads off the grammar: `<xeu>{xeunicode}` completes the pair, so `\u`+`\u`,
`\U`+`\U` and either mixture pair; `<xe,xeu>{xeunicodefail}` is live there too, so a
**malformed** `\u` / `\U` while a half is pending is that rule's `22025`
(`E'\uD83D\u12'`) and not the pairing error; and `<xeu>.`, `<xeu>\n` and
`<xeu><<EOF>>` take everything those two leave as
`42601 invalid Unicode surrogate pair`. The `<xe>`-only escape rules do **not** run in
`xeu`, so a backslash reaches those three too: an escape that is not `\u` / `\U` is taken
there alongside a plain character, a `''` and the closing quote — `E'\uD83D\x41'`,
`E'\uD83D\t'` and `E'\uD83D\\'` all answer it on 18.6.

Those are two different contrasts and the decoder spells them at two sites. That
catch-all's reach over a non-Unicode escape is what its pending guard looks ahead for — a `u`
or a `U` after the backslash rather than a backslash — because with the wider lookahead
`E'\uD83D\x41\uDCA9'` decodes to `A💩`, which 18.6 refuses. The middle rule's `22025` is
`push_unicode_escape`'s malformed arm rather than the guard's: `E'\uD83D\u12'` reaches
it under either lookahead.

In a `U&'…'` `str_udeescape` holds the half in `pair_first` and refuses it at the
doubled-escape arm, the plain-character arm and the end of the string, so `\XXXX` and
`\+XXXXXX` pair in all four combinations. The arm that does **not** read `pair_first` is
the malformed-escape one, which `ereport`s unconditionally: `U&'\D83D\zz'` and
`U&'\D83D\+zz'` are `invalid Unicode escape` on both engines rather than the pairing
error, `decode_unicode` taking its `take_hex` refusal before it consults `pending` for
the same reason.

A pair does not reach across two literals: `E'\uD83D' || E'\uDCA9'` is `42601` on both
engines, each literal being finished on its own.

**The two decoders disagree about a malformed escape's SQLSTATE, and both sides are
reproduced.** Too few hex digits after `\u` / `\U` inside an `E'…'` is
`{xeunicodefail}`, which raises `ERRCODE_INVALID_ESCAPE_SEQUENCE` — **`22025`**, pglet's
one producer of that code. The same malformation inside a `U&'…'` reaches the grammar's
own unescaper and is a plain `42601` with the same sentence. Unifying them would make
one of the two wrong. The order the value check and the pairing check run in differs
between them for the same reason: `str_udeescape` runs `check_unicode_value` first, so
`U&'\D83D\0000'` is `invalid Unicode escape value` where the `xeu` state answers
`invalid Unicode surrogate pair` for `E'\uD83D\u0000'`.

**Residuals**, all measured on 18.6:

- pglet's **`P`** for a `42601` raised inside a literal is the literal's own start where
  18.6's is the offending escape (and, for an unpaired first half, the token the `xeu`
  state fails at). The `string_escape/*` fixtures that assert a `42601` carry a
  justified `-- @no-error-fields:` for it; the `22021` and `22025` ones do not. On the
  `22021` that matches — neither engine sends a `P` there. On the `22025` it does not:
  pglet sends none and **18.6 does**, its caret on the offending escape (`E'\u00'`,
  `E'\U0000000'`, `E'\uD83D\u12'`), so the `22025` carries a residual of its own — an
  **absent** field rather than the `42601`'s misplaced one. Its fixtures pass anyway
  because the harness scores an omitted field as agreement. `H` is absent the same way
  and at the same positions: `{xeunicodefail}` carries
  `errhint("Unicode escapes must be \\uXXXX or \\UXXXXXXXX.")` and pglet sends no hint,
  measured on the same three spellings.
- `Error::Syntax`'s `syntax error: ` prefix sits ahead of every one of these `42601`
  messages, which 18.6 does not send (`EDGES.md`).
- 18.6's own error **cursor** inside a `U&'…'` is computed against the post-scanner
  buffer while its base is a raw-source offset, so a `''` collapsed ahead of the failure
  desynchronises the two; where the resulting offset cuts a multi-byte character, 18.6
  answers `22021` naming that character's bytes instead of the escape error it was about
  to raise.
  pglet answers the escape error (`EDGES.md`).
- The `UESCAPE 'c'` clause is unsupported, so a `U&'…'` here always reads `\` as its
  escape character; a literal carrying the clause falls through to the parser as
  separate tokens.
- The Unicode **identifier** form `U&"…"` is not modeled. 18.6 has it: `base_yylex`
  hands `UIDENT` to the same `str_udeescape` it hands `USCONST`, so `U&"\0041"` names a
  column `A` there and `U&"é\DCA9"` is the pairing error. Here `U&"…"` lexes as the
  identifier `U`, the operator `&` and a quoted identifier, and is refused `42883
  operator does not exist: unknown & unknown` — for those two spellings and for the
  doubled-quote member `U&"""é\DCA9"` alike (`EDGES.md` row 184) — so it is a **scope
  gap** rather than a wrong answer, loud in either direction, and it is the reason a
  divergence recorded against the string form does not carry to the identifier one
  (`EDGES.md` row 129).

## Guard

Thousands of differential fixtures compare pglet's output byte-for-byte against real Postgres
(`tests/fixtures/`), and the native fused parse+plan latency is benched
(`benches/parse_results.json`). Both are the standing regression guard.
