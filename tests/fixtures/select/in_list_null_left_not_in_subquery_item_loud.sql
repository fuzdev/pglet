-- A NULL left operand of a `NOT IN` list still evaluates a scalar-subquery item, whose
-- division raises on both engines.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
CREATE TABLE tn (x INT);
INSERT INTO tn VALUES (NULL);
-- @expect-error: division by zero
-- @test
SELECT x NOT IN (1, (SELECT 1/(a-a) FROM r)) FROM tn;
