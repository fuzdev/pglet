-- The CTE spelling of `select/unnest_over_a_derived_table_shadowing_its_source.sql`, and
-- the one a real consumer writes: `WITH t AS (SELECT arr AS a FROM t) SELECT unnest(a)
-- FROM t` names the CTE after its own source table, which 18.6 reads as the CTE
-- throughout. Pre-repair the FROM name resolved to the base table, the `unnest` gate saw
-- the base table's `a INT` rather than the CTE's `INT[]`, and the query was `42883`
-- against 18.6's `(0 rows)`.
--
-- The table is empty for the reason its derived-table sibling's is: with rows, a
-- projection-position `unnest(…)` over a FROM clause is `0A000` here on both sides of
-- the repair, and the scope's verdict would be invisible under that unrelated gap.
-- @setup
CREATE TABLE unnest_cte_shadow (a INT, b INT[]);

-- @test
WITH unnest_cte_shadow AS (SELECT b AS a FROM unnest_cte_shadow)
SELECT unnest(a) FROM unnest_cte_shadow;
