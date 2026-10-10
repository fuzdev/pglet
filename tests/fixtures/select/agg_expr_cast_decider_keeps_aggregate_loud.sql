-- A cast is no certain decider, whatever it casts: the `OR` keeps its right operand's
-- aggregate. `0::bool` is `false`, so 18.6's planner keeps the aggregate too.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT 0::bool OR sum(1/b) > 0 FROM u;
