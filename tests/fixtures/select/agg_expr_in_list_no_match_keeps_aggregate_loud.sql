-- An aggregate among an `IN` list's items is computed whatever the items before it. No
-- earlier item equals the operand here, so 18.6 keeps the aggregate too, and it runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT 1 IN (2, 3, sum(1/b)) FROM u;
