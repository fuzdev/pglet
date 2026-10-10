-- The items of an `IN` list over a `NULL` operand are still evaluated: a failing
-- constant among them raises, as 18.6's planner raises it.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT NULL::int IN (1/0, sum(a)) FROM u;
