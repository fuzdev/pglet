-- What turned the FROM-name shadow from a quiet mis-resolution into a **wrong refusal**:
-- the plan-time `unnest(<non-array>)` gate. It asks the same `ColumnScope` for its
-- argument's type, so a scope that had resolved `unnest_shadow` to the base table handed
-- it the base table's `a INT` in place of the derived table's `a INT[]`, and the gate
-- refused `42883 function unnest(integer) does not exist` — the exact message 18.6 emits
-- for the *other* statement, over a query it answers.
--
-- The table is left **empty** for a reason outside this defect: a projection-position
-- `unnest(…)` over a FROM clause is `0A000` here whatever its argument type, and that
-- refusal is raised when a row reaches the expansion — so a row-bearing spelling of this
-- statement diverges either way and pins nothing about the scope. With no rows the
-- expansion is never reached, both
-- engines answer `(0 rows)`, and the only thing left that can fail is the plan-time gate
-- — which is precisely what was wrong. `select/unnest_shadowed_arg_type_refused.sql`
-- carries the same collision at the polarity where the gate is *supposed* to fire.
-- @setup
CREATE TABLE unnest_shadow (a INT, b INT[]);

-- @test
SELECT unnest(a) FROM (SELECT b AS a FROM unnest_shadow) AS unnest_shadow;
