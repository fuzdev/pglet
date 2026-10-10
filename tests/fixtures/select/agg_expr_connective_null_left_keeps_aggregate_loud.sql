-- A `NULL` left operand decides no `AND`, so the right operand's aggregate is kept and
-- runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT NULL AND sum(1/b) > 0 FROM u;
