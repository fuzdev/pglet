-- A bound NULL operand of the parenthesized `IN` sub-select over an empty table: `NOT IN`
-- is `true`, `IN` `false`.
-- @setup
CREATE TABLE e (a INT);
-- @param $1 int4 NULL
-- @test
SELECT $1::int NOT IN ((SELECT a FROM e)) AS nin, $1::int IN ((SELECT a FROM e)) AS i;
