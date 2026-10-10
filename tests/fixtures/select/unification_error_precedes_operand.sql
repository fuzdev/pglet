-- The mirror of `operand_error_precedes_unification.sql`: swap the two conjuncts and
-- Postgres reports the other error (`42804`), because now the `COALESCE` is leftmost.
-- Both spellings must hold at once, which is what makes this a walk-order property
-- rather than a pass-order one.
-- @setup
CREATE TABLE ueo (d DATE, k TEXT, x INT4);
INSERT INTO ueo VALUES ('2026-01-01', 'a', 1);
-- @test
-- @expect-error: the leftmost error is now the COALESCE branch set (42804)
SELECT COALESCE(k, x) IS NULL AND d LIKE 'x%' FROM ueo;
