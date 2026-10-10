-- The reject polarity of the shadow, and the half where the pre-repair engine was
-- **silent**: the base table's columns are *gone* under the alias, not merely outranked
-- by the derived table's, so 18.6 answers `42703 column "a" does not exist` even though a
-- table called `shadow_hide` with a column `a` plainly exists.
--
-- Pre-repair the scope carried the base table's `a`, so the plan-time binding gate bound
-- it and the refusal was left to the executor's row resolver — which never runs without a
-- row. That is why the derived table here is **empty**: measured, this exact statement
-- answered `(0 rows)` here against 18.6's `42703`, while the row-bearing spelling
-- (`(SELECT 1 AS z)`) was `42703` on both engines all along and would pin nothing. Same
-- row-count dependence as the rest of this scope's history, in the direction that hides
-- it.
--
-- `select/derived_table_alias_shadow_hides_base_columns_qualified.sql` is the qualifier
-- tier of the same statement.
-- @setup
CREATE TABLE shadow_hide (a INT, b INT[]);
INSERT INTO shadow_hide VALUES (1, ARRAY[7, 8]);

-- @test
-- @expect-error: the alias replaced the base table, so `a` is not in scope (42703)
SELECT a FROM (SELECT a AS z FROM shadow_hide WHERE false) AS shadow_hide;
