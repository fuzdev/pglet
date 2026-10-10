-- `FROM unnest(a) AS v` names its single output column `v`, not `unnest`.
--
-- Postgres's `chooseScalarFunctionAlias` (`parse_relation.c`): with exactly one
-- function in the range-table entry and a written alias, the alias names the column as
-- well as the relation. pglet defaulted every unaliased column to the function name, so
-- this query was `42703` here and answered there — and its inverse,
-- `unnest_alias_does_not_name_unnest.sql`, answered here and was `42703` there.
--
-- It is not only a `SELECT` concern: the column name is what a **closed** scope counts
-- claimants with, so while it was wrong `UPDATE t SET … FROM unnest(a) AS v RETURNING v`
-- silently answered the target's `v` where 18.6 says `42702`
-- (`update/returning_from_unnest_source_column_ambiguous.sql`).
-- @setup

-- @test
SELECT v FROM unnest(ARRAY['a', 'b', 'c']) AS v ORDER BY v;
