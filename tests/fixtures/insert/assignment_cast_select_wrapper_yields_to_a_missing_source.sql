-- **An error-order control — do not trim it**, and the shape that says the decline has
-- to read the **whole** source plan. Its sibling
-- `assignment_cast_select_yields_to_a_missing_source.sql` puts the missing relation
-- directly under the projection, where the source-type walk reaches it and answers "no
-- type" of its own accord; here a `LIMIT` sits between them, so the walk never descends
-- to the leaf and the fallback's `Plan::output_columns` substitutes `text`.
--
-- 18.6 resolves the range table before it coerces anything (`transformFromClause` ahead
-- of the target list), so this is `42P01` there whatever the types are. pglet raises
-- that at execution, which is what lets a plan-time `42804` pre-empt it — measured on
-- this statement and on its `ORDER BY 1` / `DISTINCT` / derived-table /
-- one-side-of-a-join / set-operation spellings, all six the same way.
-- @setup
CREATE TABLE acwm (id INT, i INT);

-- @expect-error: relation "acwm_nosuch" does not exist (42P01)
-- @test
INSERT INTO acwm (i) SELECT n FROM acwm_nosuch LIMIT 1;
