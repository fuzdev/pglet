-- A NULL left operand of `NOT IN` over a correlated sub-plan does not skip it: it runs,
-- and its division raises on both engines.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
CREATE TABLE tn (x INT);
INSERT INTO tn VALUES (NULL);
-- @expect-error: division by zero
-- @test
SELECT x NOT IN (SELECT 1/(a-a) FROM r WHERE tn.x IS NULL) FROM tn;
