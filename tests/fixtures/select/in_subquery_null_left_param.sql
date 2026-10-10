-- A bound NULL left operand over an empty sub-plan: `NOT IN` is `true`.
-- @setup
CREATE TABLE e (a INT);
-- @param $1 int4 NULL
-- @test
SELECT $1::int NOT IN (SELECT a FROM e) AS nin, $1::int IN (SELECT a FROM e) AS i;
