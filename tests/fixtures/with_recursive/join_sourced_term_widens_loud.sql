-- The **reject** side of the same repair. The recursive term's column comes from a table
-- joined to the working table under an alias, so the catalog-keyed lookup answered
-- "cannot infer" and the check **skipped the column entirely** — an `int8` recursive
-- term against an `int4` seed was accepted, and this query returned two rows (`1`, `100`)
-- under a column declared `integer`, sourced from a `bigint`. Resolving against the
-- term's own input relations types `t.big` exactly, so the widening is seen and reported
-- where 18.6 reports it.
--
-- The alias is load-bearing, and not for the reason it looks like: the old scope keyed
-- the **catalog** by the written qualifier, so an unaliased `jst_src.big` resolved there
-- and was checked. Only a qualifier the catalog cannot answer to — a FROM alias, a
-- derived relation, the working table itself — fell through to "cannot infer". Measured:
-- with the alias dropped this query is `42804` on the pre-fix engine as well, so an
-- unaliased spelling pins nothing. See `with_recursive/working_table_column_typed.sql`
-- for the accept-side twin.
-- @setup
CREATE TABLE jst_src (n INT, big BIGINT);
INSERT INTO jst_src VALUES (1, 100), (2, 200);

-- @test
-- @expect-error: the recursive term widens the column to bigint (42804)
WITH RECURSIVE jst(n) AS (
  SELECT 1
  UNION ALL
  SELECT t.big FROM jst JOIN jst_src t ON t.n = jst.n
)
SELECT * FROM jst;
