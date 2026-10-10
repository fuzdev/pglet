-- A comparison with a `NULL` literal is no certain decider — it is `NULL`, not `false`:
-- `NOT (1 = NULL)` decides no `OR`, on either engine, and the right operand's aggregate
-- runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT NOT (1 = NULL) OR sum(1/b) > 0 FROM u;
