-- A `BETWEEN`'s upper bound takes only what binds tighter than `BETWEEN`, so an
-- operator binding below it — `IS NULL`, `ISNULL`, a comparison, `IS NOT DISTINCT FROM`
-- — takes the whole `BETWEEN` as its left operand (`a_expr BETWEEN b_expr AND a_expr
-- %prec BETWEEN`).
-- @test
SELECT NULL::bool BETWEEN false AND true IS NULL AS a,
       NULL::bool BETWEEN false AND true ISNULL AS b,
       1 BETWEEN 0 AND 2 = true AS c,
       1 NOT BETWEEN 0 AND 2 = false AS d,
       1 BETWEEN 0 AND 2 IS NOT DISTINCT FROM true AS e;
