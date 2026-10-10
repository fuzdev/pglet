-- The `GROUP BY` half of the coercion **asymmetry**. The projection is type-resolved
-- and `GROUP BY` was not, so `(x = '1')` became `x = '1'::int4` in the select list and
-- stayed `x = '1'` in the grouping key. `validate_grouped_expr` matches a projected
-- expression against its `GROUP BY` key by structural `==`, so the two spellings of
-- the same expression no longer matched and pglet raised a spurious `42803`
-- ("column x must appear in the GROUP BY clause") on a query Postgres runs.
--
-- The fix is not "coerce GROUP BY" but "coerce both against one scope": either clause
-- resolved alone reintroduces the mismatch from the other side.
-- @setup
CREATE TABLE gt (x INT, tag TEXT);
INSERT INTO gt VALUES (1, 'a'), (1, 'b'), (2, 'c');
-- @test
SELECT (x = '1') AS e, count(*) AS n FROM gt GROUP BY (x = '1') ORDER BY e;
