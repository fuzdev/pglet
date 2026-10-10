-- `COALESCE` stops evaluating at its first non-`NULL` argument, but a later argument's
-- aggregate is computed anyway: `max(a)` is never `NULL` here, and the `sum` still
-- divides. Computing an aggregate only where `COALESCE` reads it would answer `2`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT coalesce(max(a), sum(1/b)) FROM u;
