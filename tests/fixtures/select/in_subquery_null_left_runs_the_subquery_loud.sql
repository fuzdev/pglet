-- A NULL left operand does not skip the sub-plan: it runs, and its division raises on both
-- engines.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
CREATE TABLE tn (x INT);
INSERT INTO tn VALUES (NULL);
-- @expect-error: division by zero
-- @test
SELECT x IN (SELECT 1/(a-a) FROM r) FROM tn;
