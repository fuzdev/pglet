-- An `IN` list is no certain decider: its first item matches the operand, but the item
-- after it is no constant, so 18.6 does not fold the list and the `OR` keeps its right
-- operand's aggregate. Taking the list for a constant `true` would answer `t`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT 1 IN (1, count(*)) OR sum(1/b) > 0 FROM u;
