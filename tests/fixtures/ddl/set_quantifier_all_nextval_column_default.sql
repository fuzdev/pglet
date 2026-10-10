-- The column `DEFAULT` site has **two** parsers, and this is the accept polarity at the
-- one no other `set_quantifier_*` fixture reaches (its unquantified spellings are all
-- over the corpus): `parser::ddl`'s classifier, which recognizes a bare
-- `DEFAULT nextval('s')` by tokens and parses that call's argument list itself. It read
-- no set quantifier, so the bare spelling was `42601` here against a plain accept on
-- 18.6 — and disagreed with its own parenthesized spelling, which leaves the classifier
-- for the general path and accepted it.
--
-- The three spellings are compared in one `@test` because the property is that they are
-- **one stored fact**, which is `ddl/default_parenthesized_nextval_renders_like_the_bare_form.sql`'s
-- claim for the unquantified pair and this fixture's for the quantified one. That is
-- assertable here — `column_default` is byte-identical across the three and across both
-- engines — where the `upper(ALL 'a')` sibling (`set_quantifier_all_column_default.sql`)
-- must assert a backfilled value instead, since the two engines render *that* stored
-- expression differently.
--
-- The property is asserted for the argument spelling written here — a bare name — and it
-- is **not** a property of the site: the classifier runs no argument transform (it cannot,
-- since pg_dump's `nextval('x'::regclass)` must load), so a *constant cast* splits the two
-- spellings apart. `DEFAULT nextval(ALL 'x'::int)` is accepted here and mints `x` while
-- its parenthesized twin is `22P02`, which is 18.6's answer to both — the same split the
-- unquantified cast spelling already had. Recorded in `EDGES.md`;
-- unfixturable here, since an `@expect-error` needs a pglet error.
--
-- The `::text` casts are the corpus idiom for this query: `information_schema.columns`
-- declares `name` (OID 19) and `character_data` (a domain over `varchar`) where pglet
-- models neither and reports `text`, so projecting the raw columns would need a
-- `@no-declared-types:` waiver over a gap that is not this fixture's subject.
-- @setup
CREATE SEQUENCE sq_nad;
CREATE TABLE sq_nad_bare (a BIGINT DEFAULT nextval(ALL 'sq_nad'));
CREATE TABLE sq_nad_paren (a BIGINT DEFAULT (nextval(ALL 'sq_nad')));
CREATE TABLE sq_nad_plain (a BIGINT DEFAULT nextval('sq_nad'));
-- @test
SELECT table_name::text AS table_name, column_default::text AS column_default
FROM information_schema.columns
WHERE table_name IN ('sq_nad_bare', 'sq_nad_paren', 'sq_nad_plain') AND column_name = 'a'
ORDER BY table_name;
