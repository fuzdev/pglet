-- The qualifier tier of `select/derived_table_alias_shadow_hides_base_columns.sql`.
-- `binding_qualified` picks the one relation the qualifier names and asks it for the
-- column; with the FROM name resolved to the base table it found `a` there and bound it,
-- and the statement answered `(0 rows)` here against 18.6's `42703`. Both tiers were
-- silent over an empty derived table and both are loud now, but they are separate walks
-- with separate verdicts — a repair reaching only one of them would leave this green-
-- looking and wrong.
-- @setup
CREATE TABLE shadow_hide_q (a INT, b INT[]);
INSERT INTO shadow_hide_q VALUES (1, ARRAY[7, 8]);

-- @test
-- @expect-error: `shadow_hide_q` names the derived table, which has no `a` (42703)
SELECT shadow_hide_q.a FROM (SELECT a AS z FROM shadow_hide_q WHERE false) AS shadow_hide_q;
