-- `HAVING` already ran the plan-time walk; what it lacked was branch *types* — an
-- aggregate call resolved to "unknown", so a `COALESCE` of two aggregates had nothing
-- to compare and passed. An aggregate's type follows its argument's and is as concrete
-- as a column's.
-- @setup
CREATE TABLE hvc (i4 INT4, tx TEXT);
INSERT INTO hvc VALUES (1, 'a');
-- @test
-- @expect-error: COALESCE of min(int) and max(text) has no common type (42804)
SELECT count(*) FROM hvc HAVING COALESCE(min(i4), max(tx)) IS NULL;
