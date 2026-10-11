# SQL parser — pglet's own native, Postgres-only front-end

pglet parses SQL with its own native parser. There's no third-party parser dependency.
SQL text lowers **directly** to the planner IR (`Expr`/`Plan`) with no intermediate AST,
so a parse is also a plan. The code lives in `crates/pglet_sql/src/parser/`: `mod.rs` is
the entry, `expr.rs` the expression engine, `select.rs`/`dml.rs`/`ddl.rs` the statements,
`with_recursive.rs` the `WITH RECURSIVE` list's pre-scan and re-parse, `plpgsql.rs` the
`DO $$…$$` block, `type_name.rs` type-name normalization, and `stream.rs` the token
cursor. `fuzz.rs` is a `#[cfg(test)]` fuzz module.

## Why native

- **Expression / partial `ON CONFLICT`.** The spine's grant upsert keys on
  `COALESCE(scope_kind,'') … WHERE revoked_at IS NULL`, an expression plus partial
  conflict target. `sqlparser`'s AST can't represent it, and that's why the parser
  exists. The native parser parses, plans and executes it.
- **Size.** `sqlparser` pulls in a `recursive → stacker → psm` C-toolchain subtree, the
  single biggest wasm-size lever. Replacing it cut the wasm from ~709 KB to ~318 KB gzip
  at the time (−55%); see `TRADEOFFS.md`.

## Design

- **Direct-to-IR (fused).** The parser emits planner `Expr`/`Plan` right away. There's
  no separate AST arena (pglet has no formatter, so a second tree would buy nothing) and
  no second traversal. Expression and cast constructs raise their SQLSTATE at parse
  time: `42601` for syntax, `42704` for an unsupported type, `0A000` for an unsupported
  construct. A DDL column type name passes through and resolves at plan time, failing
  loud there.
- **Recursive-descent + Pratt.** Statements parse by recursive descent. Expressions use
  a Pratt core: a single `parse_expr_bp(min_bp)` loop over a *pure* binary-operator
  binding-power table. The keyword-led forms that aren't clean binary operators (`NOT`,
  the `IS …` postfix family, `BETWEEN`/`IN`/`LIKE`, `::`, …) are explicit productions. A
  recursion depth guard bounds expression nesting, and the tree the parser builds is
  bounded as it's built (`pglet_sql::depth`).
- **Streaming lexer + 2-token cursor.** The lexer is pull-based, and `stream.rs` is a
  fixed two-slot lookahead window. SQL is LL-friendly, so 1–2 tokens are enough and no
  byte-scan hacks are needed. **One construct needs more, and it uses a restore point
  instead of a wider window.** A *multi-word* typed constant (`DOUBLE PRECISION '1.5'`,
  `NUMERIC(5,2) '1.5'`) is a type name of unbounded length followed by a string, and
  every one of its lead words is also a legal bare column name. `TokenStream`'s
  `checkpoint`/`restore` saves a cloned lexer cursor plus the two-token window (the
  lexer is a borrowed `&str` and an offset, so the clone is free). That lets this one
  production be tried and put back, which is what Postgres's LALR grammar does by
  deferring the reduction. The exception is a window the reader has **committed** to:
  there a failure is the statement's answer and nothing is put back (the typed-constant
  bullet below gives the rule and which lead word it covers). Every other decision still
  fits in the window.
- **Shared lowering.** Statement parsing goes through the same `lower_select` /
  `lower_insert` / `lower_update` / `lower_delete` / `build_call` helpers the planner
  owns, so a query lowers to the canonical plan, including index selection, ORDER BY
  pushdown, and aggregate/window classification.
- **Keyword classification is the parser's job.** The lexer only recognizes spellings.
  Reserved-vs-identifier classification (`is_reserved`) lives in the parser, so a
  non-reserved keyword (a column named `sequence`, an alias `AS exists`) parses as an
  identifier.

## Scope

Modern Postgres only, sized to the spine's SQL surface:

- **DML**: INSERT, including expression/partial `ON CONFLICT`; UPDATE, including a
  target alias and `UPDATE … FROM other` joined-source updates; and DELETE.
- **DDL**: CREATE TABLE/INDEX/TYPE/SCHEMA/SEQUENCE, ALTER including ALTER SEQUENCE, DROP
  TABLE/INDEX/TYPE/SCHEMA/SEQUENCE, and TRUNCATE. `CREATE`/`DROP SCHEMA` model the single
  implicit `public` schema, for the migration-reset idiom. `CREATE`/`DROP INDEX` take
  `CONCURRENTLY` in `gram.y`'s keyword position. `CREATE`/`DROP EXTENSION` parse with
  their full option tails over a fixed name set; see [ddl.md](ddl.md) § Extensions.
- **COPY**: `COPY … FROM STDIN`/`TO STDOUT`, plus the `pg_dump`-preamble statements
  accepted as no-ops (`GRANT`/`REVOKE`/`COMMENT ON`/`OWNER TO`).
- **A minimal `DO $$ … $$` block** for the idempotent `CREATE TYPE` idiom:
  `BEGIN <stmts>; [EXCEPTION WHEN <cond> THEN <handler>;] END`. It isn't general
  PL/pgSQL. Its `EXCEPTION` clause is a subtransaction, so a caught error discards the
  body.
- **Transactions, including savepoints**: `SAVEPOINT n`, `ROLLBACK TO [SAVEPOINT] n` and
  `RELEASE [SAVEPOINT] n`. The optional noise word is itself a legal `ColId`, so
  `RELEASE savepoint` names one.
- **Full SELECT**: joins, derived tables, FROM-`unnest`, and CTEs, including:
  - **data-modifying CTEs** (`WITH x AS (UPDATE/DELETE/INSERT … RETURNING) … FROM x`),
    lowered to a `Plan::With` + `Plan::CteScan`. The statement after a `WITH` list must
    be a query or a data-modifying statement. Anything else is `42601` at its first
    token, as in `gram.y`, including the `TABLE` and `MERGE` spellings, which pglet
    doesn't parse anywhere.
  - a `name(cols)` column list on any of them. It's carried beside the plan and applied
    at every reader, never baked into the body (`docs/scope.md`).
  - the `MATERIALIZED` / `NOT MATERIALIZED` noise words.
  - **`WITH RECURSIVE`**, its list resolved in dependency order, each recursive CTE a
    `Plan::With` entry.

  SELECT also covers window `OVER`, GROUP BY/HAVING/ORDER BY/LIMIT, `SELECT DISTINCT` /
  **`DISTINCT ON (…)`**, `CASE`, and joins including **`CROSS JOIN`**, comma-join and
  **`JOIN … USING`**. It also takes a trailing row-locking clause: the real
  `FOR {UPDATE|NO KEY UPDATE|SHARE|KEY SHARE} [OF …]` grammar, plus `FOR READ ONLY` as
  the clause's **first-and-only** production. `gram.y` makes these two alternatives, so
  neither order of `FOR UPDATE` + `FOR READ ONLY` is legal. The clause is honored as a
  no-op. An invalid strength fails loud. A `NOWAIT`/`SKIP LOCKED` wait policy fails loud
  too (`0A000`), deferred to the end of the statement's parse on every route, a `DO`
  body's own sub-parser included. That's the one part of the clause a lock-free engine
  can't emulate.
- **Set operations**: `UNION`/`INTERSECT`/`EXCEPT` (each with optional `ALL`) parse into
  a chained `Plan::SetOp` at Postgres precedence (`INTERSECT` binds tighter than
  `UNION`/`EXCEPT`, left-associative). A trailing `ORDER BY`/`LIMIT` binds to the
  combined result (`Limit(Sort(SetOp(…)))`).
- **Scalar special-form grammars**: `EXTRACT(field FROM x)` /
  `SUBSTRING(x FROM n FOR m)` / `TRIM([BOTH|LEADING|TRAILING] c FROM x)` /
  `POSITION(a IN b)`, the three `INTERVAL '…'` input grammars (verbose, SQL-standard
  `1-2`, ISO-8601 `P1Y2M3DT4H5M6S`), and `nextval`/`currval`/`setval` as general
  expressions (not just in `DEFAULT`) all parse.
- **Large `VALUES` INSERTs** parse in a flat iterative loop (`parse_values_rows`).
  There's no per-row recursion, so no stack-overflow risk and no separate streaming path
  ([storage.md](storage.md) § Known bottlenecks).

Per-feature end-to-end status is in [compatibility.md](compatibility.md). This list is
the parser's own scope.

## Known lexical / parse divergences

- **An operator is lexed the way `scan.l` lexes one, and refused where 18.6 refuses
  it.** Every `op_chars` run is one token (`lexer::TokenKind::Op` where pglet's grammar
  has no kind of its own for it). The run is cut where a `/*` or `--` opens inside it,
  and a trailing `+`/`-` is trimmed unless the run holds a character SQL's own operators
  don't use. So `1 =- 1` is `1 = -1` and `2 ^- 1` names the operator `^-`, both as on
  18.6. The name then reaches **analysis**, where an operator pglet implements nothing
  for is `42883 operator does not exist: <left> <op> <right>`. That's 18.6's own
  message, byte for byte where the operands are literals, casts or calls. The refusal is
  deferred to the end of the statement, so a grammar error from 18.6's raw parse still
  outranks it (`SELECT 1 ! 1 1` is `42601` on both). The divergences (the operators 18.6
  has and pglet doesn't, and what the refusal says and outranks) are in
  [compatibility.md](compatibility.md)'s *Operator lexing + resolution* row and
  `EDGES.md` row 184. Guards: `lexer::tests`'
  `the_operator_rules_are_the_reference_scanners` and `tests/fixtures/operators/`.
- **A setting's name is compared case-insensitively after the fold.** 18.6's
  `find_option` matches a GUC name through `guc_name_compare`, which ignores ASCII case
  regardless of quoting. So `SET "Quote_All_Identifiers" = on`, `RESET "Lock_Timeout"`,
  `SHOW "Search_Path"` and `set_config('Client_Encoding', …)` name the same settings as
  their lowercase spellings (we checked). `Token::name` keeps a quoted spelling verbatim,
  as the identifier rule says, so every reader of a setting's name lowercases it before
  comparing: `catalog::commands`' `SET`, `RESET` and `SHOW` readers, the `set_config` /
  `current_setting` argument, the startup packet's keys, and the `-c name=value` /
  `--name=value` assignments in its `options`. A reader that skipped this would let a
  quoted mixed-case name get past every check keyed on the name. A setting whose value
  pglet decides is decided at every one of those entry points.

  The startup packet's `options` is read the way 18.6 reads it: `pg_split_opts`, then
  `process_postgres_switches`' `getopt`, including clustered switches and switch
  arguments. Its settings are applied in 18.6's order, `options` first, and each one
  pglet models is checked where it's read. pglet's own refusal is a `FATAL`, judged on
  the value the startup ends with (`pglet_server`'s `startup` module):
  `quote_all_identifiers=on` is `0A000`, as its `SET` is, and so is a `search_path`
  naming a schema pglet doesn't have.
  Guards: `tests/fixtures/commands/set_guc_quoted_mixed_case_*.sql`,
  `show_guc_quoted_mixed_case_name.sql`, `set_local_guc_quoted_upper_case_loud.sql`, and
  `pglet_server`'s `startup_reads_its_settings_as_18_6_does` and `startup` module tests.
- **A `--` comment ends at a line feed or a carriage return**, as in `scan.l`
  (`non_newline` is `[^\n\r]`). So SQL after a lone carriage return on a comment's line
  is read: `SELECT 1 --c<CR>+ 1` is `2` on both engines, and
  `DELETE FROM t -- c<CR>WHERE a = 1` deletes one row. The statement splitter and the
  transaction-exit classifier get their tokens from the same lexer, so they agree.
  Guards: `lexer::tests`' `a_line_comment_ends_at_a_carriage_return` and the
  `comment_carriage_return` fixtures under `tests/fixtures/select/` and
  `tests/fixtures/delete/` (`EDGES.md` row 174, closed). The ledger records the
  parse-level rows: 175 (a later statement's raw-parse failure in a multi-statement
  `Query`), 176 (a later statement's error position), and 185 and 186 (a chained `LIKE`,
  and `ANY`/`ALL` on a `LIKE`-family operator's right).
- **A zero-length delimited identifier is a scanner error, in every position.** `""`
  isn't a name anywhere. Postgres refuses it in the scanner, so it's
  `42601 zero-length delimited identifier at or near """"`, not a name error.
  (`42602 invalid_name` is the plausible guess, and it's wrong; we checked.) pglet's
  `Lexer::scan_quoted_identifier` refuses it, and that's the only place that does. On
  18.6 we checked the relation name, a column definition, an output alias, a `FROM`
  alias, a column reference, a qualified column, and the `SET`/`SHOW`/`DEALLOCATE`
  targets: all `42601`. A **doubled** quote is a different token and stays legal (`""""`
  is the one-character name `"`). A zero-length **string** isn't an identifier at all:
  `SET search_path TO ''` and `set_config('search_path', '""', false)` are accepted on
  both engines, which is why the `search_path` decider still admits an empty element
  from its string reader. Guards: `lexer::tests`'
  `a_zero_length_delimited_identifier_is_a_scanner_error` and
  `tests/fixtures/commands/zero_length_identifier_*`.
- **Identifier case folding.** pglet applies Postgres's rule. An unquoted identifier is
  ASCII-downcased where the parser turns a token into a name, a `"…"`-quoted one is kept
  exact, and every comparison after that is a plain byte comparison. **An identifier is
  folded once, where it's read, and compared exactly from then on.** The alternative,
  storing the verbatim spelling and rebuilding case-insensitivity at hand-written sites,
  produces divergences in **both** directions from one cause (we checked). Too strict:
  `CREATE TABLE Foo (A INT); SELECT a FROM foo` is `42P01`, and `SELECT 1 AS Bar` labels
  its column `Bar` where Postgres says `bar`. That one is **silent**: a client keying
  rows by name reads a different key. Too loose: a case-insensitive site can't tell
  `"PUBLIC"` from `public`, so `DROP SCHEMA "PUBLIC" CASCADE` **drops the real `public`
  schema with every table in it** where 18.6 answers `3F000`. Likewise an
  `ON CONFLICT (userid)` arbiter matches an index over `"UserId"` and runs its
  `DO UPDATE`, and `SELECT "NOW"()` returns a timestamp. The fold decides which SQL text
  reaches a persisted name, so it's a store-magic fact (`PGMH` — [storage.md](storage.md)
  § Store versioning).

  `lexer::Token::name` is the fold for a token in identifier position, and the parser's
  name-producing reads go through it: `expect_name`, `expect_col_label`,
  `parse_identifier_primary` and its dotted parts, `parse_table_factor`,
  `expect_type_word`, the PL/pgSQL condition name, and the `EXTRACT` field word. So
  `Foo` / `foo` / `FOO` are one name and `"Foo"` is another. The contextual-word matchers
  (`at_ident`/`eat_ident`/`expect_ident`, and `try_function_default`) fold in place
  instead of through it, and match only an unquoted `Identifier` token.
  `parse_copy_header_arg` also folds in place, but it deliberately folds a quoted token
  too, because a `HEADER` argument is a value, not a name. Its comment lists the
  measured pairs, which diverge from 18.6 in both directions.

  A **keyword** token folds too, since Postgres's scanner downcases a word before
  classifying it: `CREATE TABLE Sequence` creates `sequence`. A **value grammar** isn't
  a name and keeps its own case-insensitivity: `'TRUE'::bool`, `INTERVAL '1 YEAR'`,
  `AT TIME ZONE 'utc'`, `EXTRACT("YEAR" FROM …)` (quoted, and it still resolves on 18.6,
  we checked), and `COPY … (HEADER "ON")` (likewise). A word Postgres *downcases and then
  compares* (a function name, an access method, an option word, a `COPY` format) is a
  name and follows the rule.

  The fold is **ASCII-width**, because the oracle folds `A`–`Z` and nothing else. We
  checked that, and it matches the source: `scansup.c`'s `downcase_identifier` folds a
  high-bit byte (through `tolower`) only when the database encoding is **single-byte**.
  That's a property of the database's encoding, which is UTF-8 on the oracle, and pglet
  models no per-database encoding. The post-parser sites that re-fold a catalog type key
  (`executor::ddl::execute_create_type`, `execute_drop_type`,
  `executor::type_resolution::resolve_column_type`) use the same width, so none of them
  folds a letter the parser passed through unchanged. With no quoting anywhere, that
  gives: `CREATE TYPE CAFÉ AS ENUM ('a'); CREATE TYPE café AS ENUM ('b')` registers two
  types on both engines, and `CREATE TYPE ZÄP AS ENUM ('a'); CREATE TABLE zt (c zäp)` is
  `42704` on both. Under a Unicode `to_lowercase()` at any of the three sites, the first
  pair collapses to one type and the second is silently accepted
  (`tests/fixtures/identifier/enum_type_name_*`). The general lesson: **a re-fold is
  only safe at the width of the fold that produced the name it re-folds**. A Unicode fold
  at a re-fold site isn't a stricter version of the same rule; it's a different rule.
  The value grammars keep their own width and aren't covered by this: `lower()`,
  `upper()`, `ILIKE` and `parse_bool` all fold Unicode here.

  A **name literal**, the `regclass` argument of `nextval`/`currval`/`setval`, folds by
  the same rule inside `SplitIdentifierString`'s grammar (`split_name_literal`), so the
  two name positions always name the same relation: `CREATE SEQUENCE MySeq` and
  `nextval('MYSEQ')` agree, and `nextval('"MySeq"')` doesn't find it.

  `tests/fixtures/identifier/*` pins the rule in both directions, plus
  `crates/pglet/tests/case_folding.rs` for the cases a differential fixture can't
  express: a refusal whose point is what it did *not* destroy, and the spellings where
  pglet's SQLSTATE differs from 18.6's.

  A **value function** needs more than the name. It's matched as a keyword, not as a
  `pg_proc` entry, so `parse_identifier_primary` skips `SqlValueFunction::from_name` for
  a quoted word. That function's arms are the five temporal names (`current_date`,
  `current_timestamp`, `current_time`, `localtime`, `localtimestamp`) plus the six
  session ones: `current_user`, `session_user`, `user`, `current_role`,
  `current_catalog` and `current_schema`. `SELECT "current_timestamp"` is `42703` on
  both engines, and dropping the `!quoted` guard makes it resolve here.
  `SELECT "now"()` names a real function and resolves on both.

  **A following `(` doesn't get the same answer across that set.** Ten of the eleven
  take no argument list and give `42601` (`current_user()`, `current_date(…)`, and the
  `CURRENT_TIMESTAMP(p)` precision form pglet doesn't model). `current_schema` is the
  one member 18.6 has in both forms: a TYPE_FUNC_NAME_KEYWORD that is *also* `pg_proc` 1402.
  So `current_schema()` falls through to `build_call` and resolves, while
  `current_schema(1)` is that call's `42883`. The split lives in
  `SqlValueFunction::takes_no_argument_list`, on the enum rather than at the parse site,
  so a member added later decides it where the member is declared.

  The **six** session names that are *not* keywords there stay out of the enum on
  purpose, and pglet agrees with 18.6 on all of them: bare `current_schemas`,
  `current_database`, `version`, `current_setting`, `pg_typeof` and `pg_backend_pid` are
  each `42703` on both engines, because a bare word that isn't a keyword is a column
  reference. Their **called** spellings are ordinary `build_call` arms (`version()`,
  `current_database()`, `current_schema()`, `current_schemas(bool)`,
  `current_setting(name[, missing_ok])`). `pg_typeof` and `pg_backend_pid` aren't
  modelled and stay loud.

  **Residual divergences**, all checked on 18.6:
  - A **quoted type name** doesn't parse at all (`42601`), where 18.6 reads it as a name
    and answers `42704 type "INT" does not exist`. See `expect_type_word` for why
    admitting the token isn't enough by itself.
  - An **ENUM name is ASCII-downcased again where it's stored**, so `CREATE TYPE "Loud"`
    followed by `CREATE TABLE t (m loud)` is accepted here and `42704` there. At that
    width the re-fold only affects a quoted name, since an unquoted one arrives from
    `Token::name` already folded. The line above is why it's kept: no type reference
    here (a column definition or a cast) can spell a quoted type name, so an enum stored under `Loud` couldn't be named
    by any column definition and couldn't be dumped. The fold rescues a name that differs
    from a bare identifier only in ASCII case and leaves the rest alone.
    `CREATE TYPE "Mood-X"`, the unquoted `CREATE TYPE Sequence`, and the unquoted
    `CREATE TYPE MÖÖD` (stored `mÖÖd`) are each accepted by both engines, and each makes
    `pglet_server dump` exit non-zero having emitted nothing (`EDGES.md`).
  - A **quoted lowercase function name** that pglet models as a function but 18.6 has
    only as grammar resolves here, where 18.6 answers `42883`. We checked
    `"coalesce"(1,2)`, `"nullif"(1,2)`, `"greatest"(1,2)`, `"least"(1,2)` and
    `"trim"(' x ')`. The spellings backed by a real `pg_proc` entry agree
    (`"substring"('abcdef',2,3)`, `"count"(*)`, `"now"()`, `"upper"('a')`,
    `"lower"('A')`, `"gen_random_uuid"()`), and so do the uppercase quoted spellings we
    tried (`"COALESCE"(1,2)`, `"TRIM"(' x ')`), which are loud on both. Closing it means
    recording in `build_call`'s table which names are `pg_proc` entries and which are
    special forms. A **set-quantifier** spelling of any of those five is loud on both
    sides, not silent, and diverges the other way: `planner`'s `grammar_only_name`
    refuses it with `42601` (the code 18.6 gives the *bare* spelling), against 18.6's
    `42883` for the quoted one. Checked: `"coalesce"(DISTINCT 1,2)`,
    `"trim"(ALL ' x ')` and `"trim"(DISTINCT ' x ')`. That's why `trim` is in that set
    and `substring` isn't. `pg_proc` has `substring` and not `trim`, so the quoted
    `substring` spellings resolve on 18.6 and pglet's ordinary answers already agree
    (`"substring"(ALL 'abcdef',2)` returns `bcdef` on both,
    `"substring"(DISTINCT 'abcdef',2)` is `42809` on both). Without `trim` in that set,
    `"trim"(ALL ' x ')` would return `x` here, a **silent** case of this gap instead of a
    loud one.
  - A quoted `"EXCLUDED".c` is `42P01`, as on Postgres. It's decided in
    `EvalEnv::resolve_outer_slot`, not in the planner scope's qualifier test. That chain
    declines a qualifier no frame owns instead of falling back to a bare match, which
    would let any unknown qualifier in that action read the `excluded` row's column of
    that name.
  - **No `NAMEDATALEN` truncation.** This is self-consistent within pglet, so both name
    positions still agree *here*. Self-consistency isn't agreement with 18.6, and the
    **No 63-byte (`NAMEDATALEN`) identifier truncation** entry below covers that more
    precisely.

  The *unquoted* `3F000`-vs-`42P01` split for a foreign schema qualifier is a separate,
  live divergence. It needs a statement context and has nothing to do with quoting. The
  DDL forms we checked (`CREATE TABLE noschema.qq`, `DROP TABLE`,
  `CREATE INDEX … ON noschema.t`, `ALTER TABLE`, `CREATE SEQUENCE`, `TRUNCATE`) are
  `3F000` on 18.6 and `42P01` here, while the DML lookups are `42P01` on both. See
  `EDGES.md`, which has the record and the forms that answer a third way.
- **A typed constant is a cast, and an identifier followed by a string literal is
  always one, read at a *different type-name production*.** `gram.y`'s `AexprConst` has
  four productions: `ConstTypename Sconst`, `func_name Sconst`,
  `func_name '(' typmod ')' Sconst`, and `ConstInterval Sconst opt_interval`. pglet
  lowers the first three through the same node `'…'::type` lowers to
  (`Parser::try_typed_constant` → `build_cast`). So the value, the declared type OID,
  the input-conversion SQLSTATE, the enforcement of a **spelled** typmod, the unaliased
  output column name and the deparse all come from the cast path, not from a second
  implementation. 18.6's own `pg_get_constraintdef` renders a stored
  `CHECK (d > DATE '…')` as `(d > '…'::date)`, which is what that shared lowering
  already produces.

  **`ConstTypename` differs from `Typename` in more than one rule.** It admits neither
  `GenericType` nor either `ConstInterval` arm, and `Typename`'s array bounds and `SETOF`
  are outside it. **The part pglet carries** is the fixed-length default, which `gram.y`
  gives in its own words as `ConstCharacter`'s reason to exist: "to allow defaulting
  fixed-length types such as CHAR() and BIT() to an unspecified length … this makes no
  sense for constructs like CHAR 'hi'". `CharacterWithoutLength` installs `bpchar(1)`,
  and `ConstCharacter` clears it again. pglet splits at the resolver instead of in the
  type-name reader. There's one `executor::type_resolution::resolve_type_name` with two
  entries, and the constant one skips that default and nothing else. So `char 'ab'` and
  `character 'ab'` are `ab`, while `'ab'::char` and `CAST('ab' AS char)` are `a`, and a
  spelled `char(1) 'ab'` is `a` again; each checked on both engines. A `char` **column**
  isn't a fourth case: `'ab'` into one is `22001` on both engines (18.6 naming
  `character(1)`, pglet `character varying(1)`), because assignment to `bpchar(n)` isn't
  an explicit cast.

  **The test is "parenless", not "no length survived the read".** `ConstCharacter`
  clears the `CharacterWithoutLength` arm and passes `CharacterWithLength` straight to
  `typenameTypeMod`. So a length the *type* refuses (below 1, or above 10 485 760) is a
  spelled modifier at *both* positions and gets the default at neither. `char(0) 'ab'`
  and `'ab'::char(0)` are both `22023 length for type char must be at least 1` on both
  engines, and `CREATE TABLE q (c CHAR(0))` builds no table, neither a `char(1)` nor an
  unconstrained column. The two other ways to key it are wrong in opposite directions.
  Read as "no modifier was spelled", the decline gets the default and caps at 1 a length
  nobody wrote. Read as "no cap", it builds the column that stores `'abcdef'`. The
  refusal is what removes the third state both readings need (`docs/coercion.md` § A
  modifier the type refuses, `EDGES.md` row 124). The `types/refused_typmod_*` fixtures
  pin this differentially at both positions, and
  `crates/pglet/tests/typmod_decomposition.rs` pins it end to end: the values, and the
  catalog a refusal must not leave behind. (The **writes** in that file belong to the
  *polarity* half, over modifiers the refusal must not reach; a refusal has no write to
  test.) The `varchar` branch of the same arm spells no default at either position, so
  it can't see the split.

  **A number the window refuses splits the two positions**, and the cause is the
  speculative restore, nothing about `char`. `commits_on_open_paren` admits `float`
  alone, so at every other carrier lead word, a `(…)` that fails the reader's own window
  restores the speculation and falls back to the function-call path. The restored cursor
  then decides the answer: `0A000 function not supported: <word>` where the lead word is
  the whole spelling, or a `42601` from `SELECT`'s own grammar where a tail is left over
  (`character varying`, the one spelling whose SQLSTATE agrees). So `char(3.5) 'ab'`,
  `char(2147483648) 'ab'`, `char(99999999999999999999) 'ab'` and their `character` /
  `varchar` / `character varying` forms are that `0A000` / `42601`, against 18.6's
  `42601`. Their **cast** spellings agree at `42601`. `char(-1) 'ab'`, `char(5,2) 'ab'`,
  `numeric(+5) '1'` and the `timestamp(3.5) '…'` / `time(3.5) '…'` pair are cases too,
  while the three number shapes above aren't cases at `numeric`, where they agree.
  `EDGES.md` rows 116 and 124 hold the derived list of cases. The `0A000` at a
  *modifier-bearing* uncommitted lead word isn't the only thing the constant position
  gets wrong here.

  Three of the five spellings that 18.6's `character:` production maps to `bpchar`
  aren't modelled here, so they're inert. `nchar` is `42704` at both positions, and
  `national char` / `national character` is `42704` at a declared position and `42601`
  at a constant one (the multiword lead is outside the speculation set, below).
  `ConstBit`'s part is inert the same way: pglet models neither `bit` nor `varbit`, so
  `bit '101'` is `42704` before a default could be suppressed. So is `timetz`'s part of
  the temporal-precision rule. The constant position is a case of `EDGES.md` row 95. It
  declares 1043 against 1042, `column_default` keeps a bare
  `character varying`-against-`bpchar` name gap where the other three catalog renderings
  differ by a whole extra coercion node, and 18.6's blank-insensitive `bpchar` can be
  reached through a constant that carries trailing blanks. Row 95 holds the measured
  cells.

  There are two **dispatch shapes**, because only one can be decided from the two-token
  window. An identifier immediately followed by a string is always a typed constant.
  18.6 answers `SELECT foo 'x'` with `42704 type "foo" does not exist` even when `foo` is
  a real column, so committing is safe, and it's what keeps a bare `date` resolving as a
  column while `date 'x'` doesn't. A **multi-word or modifier-bearing** type name
  (`DOUBLE PRECISION '…'`, `CHARACTER VARYING '…'`, `TIMESTAMP WITH TIME ZONE '…'`,
  `NUMERIC(5,2) '…'`) is different: every one of its lead words is a legal bare column
  name on both engines. So it parses speculatively from a `TokenStream::checkpoint` and
  restores the cursor when no string follows, unless the window is one it has
  **committed** to (below).

  The type name is the **scalar** production only: `int4[] '{1,2}'` is `42601` on 18.6
  (the array spelling is `_int4 '{1,2}'`, which pglet answers with `42704`).

  Residuals, all checked: a **quoted** type name still doesn't parse (`"date" '…'` is
  `42601` here against a value there, the same `expect_type_word` gap the bullet above
  records for `::"int4"`), and neither does a **schema-qualified** one
  (`pg_catalog.date '…'`). The conversion is also folded during the fused parse+plan,
  so `SELECT DATE 'x' 'y'` reports the literal's `22007` where 18.6 reports the trailing
  token's `42601`. That phase-order divergence has the same shape as `SELECT 1/0 'y'`,
  so this grammar didn't introduce it, and it's the same one a boolean gate's verdict
  hits ahead of the end-of-input check (`EDGES.md` row 128).

  The speculative path keeps its two phases separate for a reason. Reading the type
  name is the *question* and resolving it is an *answer*, and running both under one
  `if let Ok(…)` would swallow the second. `SELECT time with time zone '10:00'` would
  then report `42601` where the single-word `timetz '10:00'` reports the `TIMETZ` rule's
  `42704`: one type giving two codes depending on how it's spelled.

  **The speculation also has a third outcome: a window the reader commits to.** Where a
  lead word followed by `(` can begin nothing but a type name, a failure inside the
  speculation is the statement's answer and the cursor isn't put back. `parser::expr`'s
  `commits_on_open_paren` is the membership test, and `float` is what it admits. That's
  because `opt_float` refuses `FLOAT(0)` and `FLOAT(54)` *in the grammar* on 18.6, and a
  restore would swallow that `22023` and demote it to the function-call path's
  `0A000 function not supported: float`, the same kind of demotion the two-phase split
  describes. Committed, `SELECT float(0)` is the `22023`, and `float(1)`,
  `float(1) + 1`, `float(1.5)`, `float(1,2)` and `float(2147483648)` are `42601`, each
  agreeing with 18.6. **The `(` decides the window, not the lead word**: bare `float` is
  an ordinary column reference on both engines (`42703 column "float" does not exist`)
  and still restores. 18.6 splits the same way for the same reason: `float` is a
  `COL_NAME_KEYWORD`, excluded from `func_name`, so `float (` can begin nothing but
  `Numeric`. pglet's membership is deliberately narrower than that reason allows.
  `EDGES.md` row 116 lists the lead words that pass the same `kwlist.h` test and stay
  uncommitted, and the ones that fail it and must never join.
  `NATIONAL CHARACTER [VARYING]` is the lead word the reader still can't follow, so it's
  *not* in the speculation set: `SELECT national character '5'` is a value on 18.6 and
  `42601` here.
- **A `'(' Iconst ')'` modifier is read by one reader, and the type decides what the
  number *means*.** `gram.y` spells the type-modifier windows pglet reads with a single
  `Iconst`: `ConstDatetime`'s `(p)`, `SimpleTypename`'s `ConstInterval '(' Iconst ')'`,
  `CharacterWithLength`'s `character '(' Iconst ')'`, and `opt_float`'s `(n)`. So
  `Parser::opt_precision` serves each of them. An `ICONST` is an unsigned integer that
  fits `int32`, which is what makes `timestamp(3,1)`, `timestamp(3.5)`, `timestamp(-1)`,
  `timestamp(2147483648)` and the same four spellings of `char(n)` / `varchar(n)` each
  `42601` on both engines. Expecting `)` after one number covers the comma, and the
  `i32` parse covers the rest. Reading the **character length** through a weaker window
  would let `'abcd'::char(3.5)` resolve to an unconstrained `varchar` and return `abcd`,
  or, with the type refusing an out-of-window length, give that refusal's `22023` about
  a number the grammar never admits. The type decides what the number means, not this
  reader: a temporal `(p)` is clamped to `6`, a character length is refused outside
  `1 ..= 10485760`, and `FLOAT(n)` uses it to choose `real` or `double precision`. That
  last one is decided by this reader's caller, because `opt_float` decides it in the
  grammar (`docs/coercion.md`). `numeric(p[,s])` is deliberately **not** one of them.
  Its modifier is an `expr_list` read by `passthrough_type` and coerced to `int4` at the
  resolver ([coercion.md](coercion.md) § A modifier the type refuses).
- **An interval qualifier sets a bare number's unit, then masks the result. Whether the
  qualifier is safe to apply depends on the *number of unlabelled fields*, not on
  range-vs-single-field.** `ConstInterval Sconst opt_interval`'s trailing field word is
  part of the literal, not an output alias. Read as an alias, `INTERVAL '90' MINUTE`
  gives ninety *seconds*: a wrong value, not a parse error.
  `Parser::parse_interval_qualifier` reads the six single-field spellings. A literal
  that is a bare signed decimal is **re-decoded** with the qualifier's unit appended
  (`interval_from_unit` passes `90 minute` to the one interval decoder instead of doing
  the arithmetic a second time). The result is then masked to that field
  (`AdjustIntervalForTypmod`: fields right of the named one are zeroed and fields left
  of it kept, so `'1 day' MINUTE` is still `1 day`). `WEEK` is deliberately absent,
  because it's absent from `opt_interval` too.

  **A qualifier changes how every *unlabelled* field is read, and that isn't specific
  to the `X TO Y` forms.** `DecodeInterval`'s `DTK_NUMBER` case gives an unlabelled
  number the field the range names and then moves to the next coarser one, so a run of
  them decodes positionally: `INTERVAL '1 2' HOUR` is `1 day 02:00:00` on 18.6 under a
  **single** field word. pglet doesn't model a positional decode, and the shapes that
  would need it are refused instead of answered. A second unlabelled field is `22007` in
  the decoder itself. That's 18.6's answer for every such literal except `'1 2' HOUR`,
  and it also closes the same wrong value in the *unqualified* `INTERVAL '1 2'`. The
  `X TO Y` ranges are `0A000`. `INTERVAL(p) '…'` and a `SECOND(p)` precision are also
  unparsed and loud. Residual: `INTERVAL '1 2' HOUR` is `22007` here against a value
  there.
- **`x IS [NOT] TRUE / FALSE / UNKNOWN`** is Postgres's `BooleanTest`, with its own IR
  node (`Expr::BooleanTest`) instead of a lowering to `IS [NOT] DISTINCT FROM`. 18.6
  renders a stored `CHECK (b IS TRUE)` back as `CHECK ((b IS TRUE))`, and the operand
  gate's message names the spelling. `UNKNOWN` isn't in the keyword table, so it stays
  usable as a column name. `IS` binds tighter than `NOT`/`AND` and looser than a
  comparison, so `a = b IS TRUE` groups as `(a = b) IS TRUE`.
- **Adjacent string-literal concatenation** across a newline (`'a'\n'b'` → `'ab'`, the
  SQL-standard rule) isn't done. It fails loud, except where the general `SET` value
  reader lets it through, which drops the value (`EDGES.md` row 256).
- **No 63-byte (`NAMEDATALEN`) identifier truncation.** pglet accepts identifiers of any
  length, where 18.6 truncates each to `NAMEDATALEN - 1` = 63 bytes as the parser reads
  it, with a `42622` **notice** rather than silently. Inside pglet, the cost isn't the
  long name it stores. It's that a name 18.6 resolves is `42703` here (a 64-byte
  reference to a 63-byte column), and that two names differing only past byte 63 are
  **one** column there and two here. So any rule that reads a name list as a *set* reads
  a different set on each engine. Outside pglet it costs the dump: two declared
  relation, column or sequence names that share their first 63 bytes are two objects
  here and one name on 18.6, so the script restores short there with `psql` exiting `0`.
  We checked this for two tables with rows, two sequences and a two-column table
  (`EDGES.md` row 278's residual holds the constraint-name case). Over
  `tt (<63 × 'c'> INT, x INT)` with two such names, the `INSERT` column list is `42701`
  on 18.6, the `UPDATE` `SET` list `42601` and `COPY` `42701`, and all three are `42703`
  here ([names.md](names.md) § What the refusals do not carry).

  It also decides the **size of an error message**, and over the whole *statement*,
  18.6's side is the bigger one. Its `column "…" does not exist` carries the truncated
  63 bytes where pglet's carries the whole name: 163 B against 200 053 B for a
  200 000-character name, counting the whole `ErrorResponse` packet as a raw v3 client
  reads it. But the truncation comes with a notice, and that
  `42622 identifier "…" will be truncated to "…"` **`NoticeResponse`** includes the
  untruncated name at 200 167 B, so the statement is **200 330 B there against
  200 053 B here**. Both halves hold for quoted and unquoted names alike, and every
  identifier position we checked behaves the same: a relation name is 163 + 200 167 B
  against 200 055 B, and an unimplemented function name 265 + 200 167 B against
  200 070 B. **What this class costs is the divergence, not the size.** A message that
  names a *value* is large on both engines: `SELECT '<200 000 digits>x'::numeric` is
  200 102 B on 18.6 against 200 071 B here. Besides identifiers, the two other spellings
  that could carry their own length don't: an operator name is bounded at `NAMEDATALEN`
  by the scanner's own rule (`lexer::core`'s `scan_op_chars`), and a numeric literal's
  refusal doesn't include it (`SELECT 1e<200 000 nines>` is 59 B here, 97 B there).


## A function-call set quantifier is 18.6's, both keywords

`func_application`'s `func_name '(' DISTINCT func_arg_list opt_sort_clause ')'`
production sets `agg_distinct`, and `ParseFuncOrColumn` refuses it for anything that
resolves to an ordinary function. That's the `if (agg_distinct)` arm, the next `if`
below the `agg_star` arm that the niladic builtins' wildcard refusal implements.
Dropping the modifier at every arm except the aggregates is **silent** throughout: a
plausible value, and no error channel anywhere. At the persisted-expression sites it
stores the call with the modifier stripped, so `column_default` and the dump render
something the user never wrote. `EDGES.md` row 51 has the measured cases of that class
and its residuals. What follows is the rule.

**The class is three rules.** (1) `DISTINCT` over a non-aggregate is `42809`. (2)
`gram.y` bounds *either* keyword. Both quantifier productions require a **non-empty**
`func_arg_list`, and the `func_name '(' '*' ')'` production combines with neither, so
the whole cross-product {`DISTINCT`, `ALL`} × {`)`, `*`} × {ordinary call, window call}
is `42601` there, checked cell by cell. (3) `ALL` is the no-op quantifier. 18.6 accepts
it on any call that resolves, aggregate or not (`gram.y`'s comment on that production
says it would like to mark the node "must be an aggregate" and has nowhere to put the
bit), so refusing every spelling with `42601` would over-reject.

The three move together. **Dropping `ALL` in the parser would create a new silent
over-acceptance**, because the names 18.6 owns in its own grammar refuse either keyword,
so `coalesce(ALL 1, 2)`, which is `42601` there, would return `1`. So the quantifier is
carried as *which* keyword (`planner::SetQuantifier`), and `grammar_only_name` refuses
both keywords over the names missing from 18.6's `pg_proc` whose call would otherwise
*return a value*: `coalesce`, `greatest`, `least`, `nullif`, and `trim`. `trim`'s bare
spelling has its own parser route and its *quoted* one doesn't. The set comes from a
per-name catalog sweep of every name the dispatch matches, not from 18.6's grammar list.
`substring` is on that list and is a `pg_proc` entry, so its quoted spellings already
agree and it must stay out. `uuid_generate_v4` is absent from `pg_proc` and stays out
too, because it's niladic: its arity check already refuses every quantified spelling
with 18.6's own code. The rule adds no persisted shape, so it isn't a store-magic fact.
`SetQuantifier` never reaches an `Expr`, and `Expr::Aggregate`'s own `distinct` field is
untouched.

**All three rules apply in the FROM clause too**, where `unnest(…)` is the one function
call pglet models (`parse_table_factor` dispatches a derived table, that `unnest`, a CTE
reference, a catalog view and a base table, and nothing else). There the two keywords
split by a *second* mechanism. `transformRangeFunction` expands a multi-argument
FROM-clause `unnest` into one call per argument only for an **undecorated** call
(`!fc->agg_distinct` is among its guards). So `ALL` expands and returns rows, while
`DISTINCT` leaves an ordinary `unnest(…)` for `ParseFuncOrColumn`: one array resolves
and is `42809`, and two resolve to no signature and are
`42883 function unnest(integer[], integer[]) does not exist`, ahead of the quantifier.
The argument transform outranks **both** of those answers here, as it does at
`build_call`. 18.6 transforms the arrays before `transformRangeFunction` looks at the
decoration at all, so `FROM unnest(DISTINCT ARRAY['a'::int]) AS u(x)` is `22P02` there,
and so is its two-array spelling, where the arity refusal would otherwise give `42883`.
That's why `refuse_from_unnest_quantifier` takes the array *expressions* and not their
count.

**A third route reads one: the DDL `DEFAULT` classifier.** `parser::ddl`'s
`try_function_default` recognizes a bare `DEFAULT nextval('s')` by its tokens and parses
that call's argument list itself, so nothing written there reaches
`parse_function_call`. `nextval` is the only arm whose call takes an argument, so it's
the only case. The niladic arms' own `expect(RParen)` is already the grammar bound
(`DEFAULT now(ALL)`, `now(DISTINCT)` and `gen_random_uuid(ALL)` are `42601` on both).
**Its two positions match the general path's**, and each was checked as a pair rather
than assumed by symmetry. The verdict runs *after* the closing paren, so
`DEFAULT nextval(DISTINCT 's' junk)` is `42601` on both, where raising it above the
`expect` would give `42809`. It runs *before* the name-literal reduction, where
`sequence_call` puts it, so `DEFAULT nextval(DISTINCT 'bad name')` is `42809` on both,
where reducing the literal as it's read would give pglet's `42602`. That keeps
`parse_column_default`'s invariant that the bare and the parenthesized spelling are one
stored fact: `column_default` reads `nextval('s'::regclass)` for bare-`ALL`,
parenthesized-`ALL` and unquantified alike, on both engines.

**The deferred name-literal reduction also decides an unquantified class**, through the
classifier rather than the quantifier. Reducing the literal *after* the closing paren
puts pglet's `42602` behind the syntax error 18.6 reports. So an unquantified spelling
whose bad token is **inside** the call (where the `)` should be, or in a cast type after
the literal) is `42601` on both engines, as its parenthesized spelling is:
`DEFAULT nextval('bad name' junk)`, `nextval('bad name'::)` and
`nextval('bad name'::123)` at each of `CREATE TABLE`, `ADD COLUMN` and
`ALTER COLUMN … SET DEFAULT`, plus the unterminated `nextval('bad name'` at the latter
two, each checked on 18.6 and on both pglet builds. The condition is *inside*, and a bad
token **outside** the call is the control that shows it:
`CREATE TABLE t (a BIGINT DEFAULT nextval('bad name');`, whose unterminated element is
the table's own column list, is `42602` on both pglet builds against 18.6's `42601`.
That's a case of the later-syntax-error class `EDGES.md` row 51 records, not of this
one. The well-formed `DEFAULT nextval('bad name')` is `42602` on **both engines**, which
shows that the reduction's *position*, not the reduction itself, decides the class.
`ddl/nextval_column_default_name_literal_yields_to_the_paren_loud.sql` pins one case of
it.

**The verdict's position is checked at three scales.** The grammar bound belongs to the
parser, stated once there and not repeated in the dispatch. The grammar-name refusal
runs **before** the arity checks, because 18.6's is a parse error that outranks
everything: `nullif(DISTINCT 1)`, wrong on both counts, is `42601` on both. The `42809`
runs **after** every check that does the job of `func_get_detail`'s signature resolution
and **before** everything that does the job of something 18.6 raises later:
`upper(DISTINCT 'a', 'b')` is `42883` on both, and so is `now(DISTINCT 1)`. That's also
why the **six niladic builtins add no case to the silent half**: `niladic_call` refuses
the argument list before anything reads the quantifier, so the non-empty list decides
it, not the modifier. `jsonb_build_object(DISTINCT 'a')` and
`nextval(DISTINCT 'bad name')` are `42809` there against pglet's own `42804`/`42602`,
which is why those two arms check the verdict inside the dispatch instead of at its
end.

**Above the signature resolution is a scale of its own, and the quantifier bounds its
reach on both sides. Written on one arm it splits the names, and written for every call
it pre-empts the DDL verdicts (below).** `transformFuncCall` transforms every argument
(`transformExprRecurse`) before `ParseFuncOrColumn` sees the call, so a **constant
cast**, which is coerced during that transform, outranks both the signature resolution
and the modifier, for every name. `SELECT upper(DISTINCT 'a'::int)` is `22P02` on both
engines, as are the `abs` / `concat` / `length` / `to_char` / `date_part` /
`starts_with` / `unnest` / `jsonb_build_object` spellings and both spellings of the same
call in a column `DEFAULT`. If the transform were written on the sequence arm alone, it
would agree for the three sequence names and give `42809` for every other. The one
checked case whose answer isn't `22P02` is `set_config(DISTINCT 'x'::int, …)`, which
returns `y` here; its discarded argument makes it silent (`EDGES.md` row 51). The
transform is one call above the dispatch (`planner::transform_call_args`) and one at the
FROM route. A measured pair fixes its position at each end. It sits **below** the
grammar-only refusal, since `coalesce(DISTINCT 'a'::int, 1)` is `42601` there where the
unquantified `coalesce('a'::int, 1)` is `22P02`. It sits **above** every arity check,
since `setval(DISTINCT 'sq'::int, 1, 2, 3)`, also wrong on the count, is `22P02` there
where `setval(DISTINCT 'sq', 1, 2, 3)` is `42883` on both.

**A typmod'd cast is a constant cast, but its modifier isn't.** That's one node kind
with one non-obvious half. `'abc'::numeric(5,2)` is the same `stringTypeDatum` call as
`'abc'::numeric`, so it's `22P02` ahead of the quantifier. The modifier itself is
applied by a *separate* `coerce_type_typmod` node that the **planner** folds
(`coerce_type` passes `inputTypeMod = -1` for every type but `interval`: "Any length
constraint will be applied later by our caller"). The pair that shows it:
`concat(DISTINCT 'abc'::numeric(5,2))` is `22P02` on 18.6, and
`concat(DISTINCT '12345'::numeric(3,1))` is `42809`, the quantifier beating the overflow
the unquantified spelling does raise (`22003`, on both engines). pglet builds a separate
`Expr::CastTypmod` for the modifier spelling, so the transform matches that node too,
and must not apply the modifier. What the arm covers:
`concat(DISTINCT 'abc'::numeric(5,2))` and its `upper` twin, the two-argument
`upper(DISTINCT 'abc'::numeric(5,2), 'b')` against the arity, and the `set_config`,
column-`DEFAULT`, FROM-`unnest` and `nextval` positions — `42809` here at each of them
except the arity case's `42883`, against `22P02` there. The truncating
`'abcdef'::varchar(2)` still returns `AB` through `upper(ALL …)` on both. `interval(p)`
reaches the arm, because pglet keeps that precision (`'x'::interval(3)` builds a
`CastTypmod`), so 18.6's own `interval` carve-out (the one type whose `inputTypeMod` is
passed through) is within this arm's reach. The modifier still isn't applied here, and
two measured spellings don't tell the difference:
`concat(DISTINCT 'xyz'::interval(3))` is `22007` and
`concat(DISTINCT '0.5 sec'::interval(0))` is `42809`, each on both engines.

**The transform runs only for a call that carries a quantifier, and that gate is a
correctness rule, not a scoping convenience.** pglet raises this whole family in its
*parser*, ahead of every executor-time statement gate, while 18.6 cooks a column
`DEFAULT` and a `CHECK` in `AddRelationNewConstraints`, *after*
`heap_create_with_catalog`. Run for an unquantified call, the transform would pre-empt
the DDL verdicts that come after it. On an ungated build we saw:

- statements whose `42704` / `42P07` / `42P16` / `42703` / `42P01` / `42601` become
  `22P02`, at a column `DEFAULT` and at a `CHECK`, through `CREATE TABLE`,
  `ALTER TABLE … ADD COLUMN`, `ALTER COLUMN … SET DEFAULT`, `ADD CONSTRAINT` and
  `CREATE INDEX`;
- `IF NOT EXISTS` skips (`upper`, `to_char`, `substring`, `trim`, and a `CHECK`'s
  `upper`);
- and, in the other direction, the within-expression source order, which inverts
  instead of closing. 18.6 cooks a stored expression left to right, so
  `CHECK (upper('a'::int) = 'A' AND nosuchcol > 0)` is `22P02` there and its mirror is
  `42703`. pglet answers `42703` to **both** spellings (its own walk raises the
  column-binding error and never the cast), and the ungated hoist answers `22P02` to
  both. One spelling agrees either way and the trade is symmetric, which is why the fix
  belongs at the silent case's own site (`EDGES.md` row 51), not at this position.

The worst of them is a **success**:
`CREATE TABLE IF NOT EXISTS <existing> (b TEXT DEFAULT upper('a'::int))` is a skip on
18.6 and here, and `22P02` under the ungated hoist, with a client's `prepare()` failing
at `Parse`. The invariant the gate keeps is that **no unquantified statement this
transform can reach changes phase or answer**. Every ordinary expression position is
`22P02` on 18.6 and here (we checked `upper`, `abs`, `length`, `concat`, `to_char`,
`date_part`, `starts_with`, `count`, `jsonb_build_object`, `nextval`, `setval`,
`unnest`, `trim`, `substring`, in a select list and in `INSERT` / `UPDATE` / `DELETE`),
because the plan-time `planner::validate_const_casts` pass already answers there. The
exception is the two sequence names, whose name argument never reaches that pass and
which `planner::sequence_call` answers with a check of its own. The gate does *not*
replace that sequence arm's check. `sequence_call` reduces its name argument to a
`String` and discards the expression, so a constant cast there is raised by that arm or
not at all. Deleting it as redundant with an *ungated* transform would bring back a
**write**: `nextval('sq'::int)` and `setval('sq'::int, 50)` returning a value and moving
the counter, against 18.6's `22P02` (`crates/pglet/tests/set_quantifier_verdicts.rs`'s
`an_unquantified_sequence_name_cast_does_not_move_the_counter`, which fails when that
arm is reverted). `ddl/if_not_exists_skip_beats_a_const_cast.sql` and
`ddl/column_type_beats_a_const_cast_loud.sql` pin both unquantified directions of the
gate itself.

A third side of the same node is loud and pglet-only. The plan-time
`planner::validate_const_casts` pass matches `Expr::Cast` but not `Expr::CastTypmod`, so
`SELECT 'abc'::numeric(5,2)` raises its `22P02` at eval, where 18.6 raises it in parse
analysis. Widening that pass is **not** the fix. It runs ahead of the range table here,
so `SELECT 'abc'::numeric(5,2) FROM nosuchrel`, `42P01` on both engines, would become
`22P02`. We saw that on a build with the kind added, and **the whole fixture corpus was
green** on it. So the argument transform takes the kind and the plan-time pass doesn't
(`planner::validate_arg_const_casts` against `planner::validate_const_casts`), and a
probe holds that split, not a gate.

**Three siblings in the same `ParseFuncOrColumn` block stay out.** They're already loud
here, for a different reason: `upper('a' ORDER BY 1)` is 18.6's
`42809 ORDER BY specified, …`, `upper('a') FILTER (WHERE true)` its
`42809 FILTER specified, …`, and `upper('a') WITHIN GROUP (ORDER BY 1)` its `42883`.
The sort argument joins the signature first, so only a name that still resolves reaches
that arm (`concat('a') WITHIN GROUP (ORDER BY 'b')` is
`42809 WITHIN GROUP specified, …`). All three are `42601` here. pglet's grammar has none
of the three productions, which is also why none can be a fixture: an `@expect-error`
compares the codes. The `ORDER BY` sibling has a **FROM-route** case too:
`FROM unnest(ARRAY[1,2] ORDER BY 1)` is
`42809 ORDER BY specified, but unnest is not an aggregate function` there against
`42601` here. Its `DISTINCT` spelling `FROM unnest(DISTINCT ARRAY[1,2] ORDER BY 1)` is
18.6's `42809` for the *quantifier* (the modifier is judged before the sort clause),
against the same `42601` here. `EDGES.md` row 51 records them as divergences. They
aren't this rule's to close because the productions are missing.

**What pins the rule.** The `set_quantifier_*` differential fixtures:

- `functions/`: one case per `build_call` arm family (math, string, binary, variadic,
  jsonb, sequence, single-argument, `set_config`, `Extract`); the quoted and
  schema-qualified spellings; the grammar names in both keywords; the FROM-clause
  `unnest` route in both keywords over both array counts, plus its bound and the
  position of its verdict against the call's closing paren; the grammar bound (six of
  its eight cells; the two window-`*` cells are checked `42601` on both and reach the
  identical parser call); the precedence pairs; and both accept directions — the
  argument transform's own, which are the pairs above and not a single statement: the
  headline (`upper(DISTINCT 'a'::int)`), a **later** argument's cast on a non-string arm
  (`concat(DISTINCT 'a', 'b'::int)`, the one a first-argument-only check would pass),
  the transform's two bounds (`coalesce(DISTINCT 'a'::int, 1)` above it,
  `setval(DISTINCT 'sq'::int, 1, 2, 3)` below it), and the FROM route's two array
  counts.
- the `sequences/`, `ddl/`, `select/`, `insert/`, `update/` and `delete/` positions.

The `ddl/` set does two things. It covers the four persisted-expression sites through
the **general** expression parser (a column `DEFAULT`, a CHECK, an index key, an index
predicate, each spelled `upper`). Separately, it covers the column-`DEFAULT` site's
**second** parser, the classifier, through `nextval`: the `ALL` accept over the
`column_default` its three spellings share; the `CREATE TABLE` and
`ALTER COLUMN … SET DEFAULT` refusals (two fixtures because the `b_expr`/`a_expr`
grammar modes split there, while `ADD COLUMN` is the same call site under the same mode
as `CREATE TABLE` and gets none); and the verdict's two positions, the second with an
unquantified control beside it. `crates/pglet/tests/set_quantifier_verdicts.rs` covers
what a fixture can't hold: the counter and the row set after a refusal, the message text
that separates this refusal from its `<name>(*)` sibling in the same error variant (at
the classifier route as well as the general one), and the quoted `"trim"` case, which
both engines refuse at different codes. The two gate directions are
`ddl/if_not_exists_skip_beats_a_const_cast` and
`ddl/column_type_beats_a_const_cast_loud`, both of which fail on the ungated hoist.

## A string literal is assembled in 18.6's units, and decoded by the grammar its scanner matches

`lexer::escapes` holds both decoders: `decode_escape` for `E'…'` and `decode_unicode`
for `U&'…'`, each called from one site in `lexer::core`'s `scan_string`. A plain `'…'`
reaches neither. Under `standard_conforming_strings` it collapses `''` and reads no
backslash at all, and a `"…"` identifier collapses `""` and reads none either.

**The unit is a byte.** 18.6's `<xe>{xeoctesc}` / `<xe>{xehexesc}` do
`unsigned char c = strtoul(…); addlitchar(c)`, so a `\xHH` / `\ooo` escape contributes
one **byte**, truncated to eight bits, and everything else contributes a character's
UTF-8. The finished literal is then validated **once, left to right**, at the closing
quote (`pg_verifymbstr`). The answer depends on the unit: `E'\xC3\xA9'` is one
character (`= 'é'` is `t`), and a lone `E'\xE9'` is `22021`, not a one-character string.
Reading in **code points** would get both wrong: two characters for the first and a
value for the second. The truncation is the same unit seen from the other end:
`E'\777'` is the byte `0xFF` and `E'\400'` a NUL, both `22021`, where an untruncated
read would name the characters U+01FF and U+0100.

Rust's UTF-8 acceptance set matches `pg_utf8_islegal`'s byte for byte (both refuse
overlongs, surrogate bytes and values above `U+10FFFF`). So the scan
(`pglet_core::encoding`'s; its other entry points are in `docs/encoding.md`) is
`str::from_utf8` plus a separate check for NUL, which Rust accepts and
`pg_utf8_verifystr` doesn't. The failure is reported at the **first** of the two, which
is why `E'\xE9\x00'` reports `0xe9 0x00` and not the NUL alone. The message is
`report_invalid_encoding`'s: the bytes from that offset, `min(pg_utf_mblen(lead),
remaining)` of them. 18.6 also caps that count at 8, but `pg_utf_mblen` can't reach that
(it answers 1, 2, 3 or 4; its 5- and 6-byte arms are `#ifdef NOT_USED`), so the cap has
no counterpart here.

**An escape the grammar doesn't match is a character, not an error.** `{xehexesc}`
needs one or two hex digits. A `\x` that fails it is matched by the shorter
`{xeescape}` instead, and `unescape_single_char('x')` returns the letter, so `E'\x'` is
`x` and `E'\xZZ'` is `xZZ`. `unescape_single_char`'s arms are `b f n r t v` (the `v` is
a vertical tab, not the letter), and its default returns the character. That makes
`E'\z'`, `E'\8'` and `E'\9'` the plain characters `z`, `8` and `9` (`{xeoctesc}` only
claims `[0-7]`, so the digits never reach it).

**Two adjacent Unicode escapes that spell a UTF-16 surrogate pair are one character.**
The rule is one function (`pair_surrogate`) that both string-literal decoders call. What
differs is what counts as *adjacent*, because 18.6 spells it as a scanner state in one
and a saved variable in the other.

In an `E'…'`, a first half puts the scanner in the exclusive `xeu` state, and its rule
set reads straight off the grammar. `<xeu>{xeunicode}` completes the pair, so
`\u`+`\u`, `\U`+`\U` and either mixture pair. `<xe,xeu>{xeunicodefail}` is live there
too, so a **malformed** `\u` / `\U` while a half is pending is that rule's `22025`
(`E'\uD83D\u12'`), not the pairing error. And `<xeu>.`, `<xeu>\n` and `<xeu><<EOF>>`
take everything those two leave, as `42601 invalid Unicode surrogate pair`. The
`<xe>`-only escape rules do **not** run in `xeu`, so a backslash reaches those three
too. An escape that isn't `\u` / `\U` is taken there, along with a plain character, a
`''` and the closing quote: `E'\uD83D\x41'`, `E'\uD83D\t'` and `E'\uD83D\\'` all give
it on 18.6.

Those are two different contrasts, and the decoder handles them at two sites. Because
that catch-all also takes a non-Unicode escape, the decoder's pending guard looks ahead
for a `u` or a `U` after the backslash, not just a backslash. With the wider lookahead,
`E'\uD83D\x41\uDCA9'` would decode to `A💩`, which 18.6 refuses. The middle rule's
`22025` comes from `push_unicode_escape`'s malformed arm, not from the guard:
`E'\uD83D\u12'` reaches it under either lookahead.

In a `U&'…'`, `str_udeescape` holds the half in `pair_first` and refuses it at the
doubled-escape arm, the plain-character arm and the end of the string, so `\XXXX` and
`\+XXXXXX` pair in all four combinations. The arm that does **not** read `pair_first` is
the malformed-escape one, which `ereport`s unconditionally. So `U&'\D83D\zz'` and
`U&'\D83D\+zz'` are `invalid Unicode escape` on both engines, not the pairing error, and
`decode_unicode` takes its `take_hex` refusal before it checks `pending` for the same
reason.

A pair doesn't reach across two literals: `E'\uD83D' || E'\uDCA9'` is `42601` on both
engines, because each literal is finished on its own.

**The two decoders disagree about a malformed escape's SQLSTATE, and pglet reproduces
both.** Too few hex digits after `\u` / `\U` inside an `E'…'` is `{xeunicodefail}`,
which raises `ERRCODE_INVALID_ESCAPE_SEQUENCE`, **`22025`**. That's pglet's only source
of that code. The same malformation inside a `U&'…'` reaches the grammar's own
unescaper and is a plain `42601` with the same message. Unifying them would make one of
the two wrong. The order of the value check and the pairing check differs between them
for the same reason: `str_udeescape` runs `check_unicode_value` first, so
`U&'\D83D\0000'` is `invalid Unicode escape value` where the `xeu` state gives
`invalid Unicode surrogate pair` for `E'\uD83D\u0000'`.

**Residuals**, all checked on 18.6:

- For a `42601` raised inside a literal, pglet's **`P`** is the literal's own start,
  where 18.6's is the offending escape (and, for an unpaired first half, the token where
  the `xeu` state fails). The `string_escape/*` fixtures that assert a `42601` carry a
  justified `-- @no-error-fields:` for it; the `22021` and `22025` ones don't. For the
  `22021` that's right, since neither engine sends a `P` there. For the `22025` it isn't:
  pglet sends none and **18.6 does**, with its caret on the offending escape
  (`E'\u00'`, `E'\U0000000'`, `E'\uD83D\u12'`). So the `22025` has a residual of its
  own, an **absent** field rather than the `42601`'s misplaced one. Its fixtures pass
  anyway because the harness scores an omitted field as agreement. `H` is missing the
  same way and at the same positions: `{xeunicodefail}` carries
  `errhint("Unicode escapes must be \\uXXXX or \\UXXXXXXXX.")` and pglet sends no hint,
  checked on the same three spellings.
- `Error::Syntax`'s `syntax error: ` prefix comes before every one of these `42601`
  messages, and 18.6 doesn't send it (`EDGES.md`).
- 18.6's own error **cursor** inside a `U&'…'` is computed against the post-scanner
  buffer while its base is a raw-source offset, so a `''` collapsed before the failure
  throws the two out of sync. Where the resulting offset cuts a multi-byte character,
  18.6 answers `22021` naming that character's bytes instead of the escape error it was
  about to raise. pglet answers the escape error (`EDGES.md`).
- The `UESCAPE 'c'` clause isn't supported, so a `U&'…'` here always uses `\` as its
  escape character. A literal carrying the clause falls through to the parser as
  separate tokens.
- The Unicode **identifier** form `U&"…"` isn't modeled. 18.6 has it: `base_yylex`
  passes `UIDENT` to the same `str_udeescape` it passes `USCONST`, so `U&"\0041"` names
  a column `A` there and `U&"é\DCA9"` is the pairing error. Here `U&"…"` lexes as the
  identifier `U`, the operator `&` and a quoted identifier, and is refused with
  `42883 operator does not exist: unknown & unknown`, for those two spellings and for
  the doubled-quote case `U&"""é\DCA9"` alike (`EDGES.md` row 184). So it's a **scope
  gap**, not a wrong answer, and loud in either direction. It's also why a divergence
  recorded against the string form doesn't carry over to the identifier one (`EDGES.md`
  row 129).

## Guard

Thousands of differential fixtures compare pglet's output byte for byte against real
Postgres (`tests/fixtures/`), and the native fused parse+plan latency is benchmarked
(`benches/parse_results.json`). Together they're the standing regression guard.
