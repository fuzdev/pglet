-- `AND` stops at a left operand that is `false`: over the row `id = 1` the division is
-- never evaluated, so the conjunction counts no row where evaluating it would be `22012`.
-- 18.6's `EEOP_BOOL_AND_STEP` steps (`ExecInterpExpr`, `execExprInterp.c`) stop at the
-- first `false`.
-- @setup
CREATE TABLE sc (id INT, tx TEXT);
INSERT INTO sc VALUES (1, 'x');
-- @test
SELECT count(*) FROM sc WHERE (id = 99 AND 1/(id-1) = 0);
