-- A `NULL` cast to the `COALESCE`'s type ends nothing: the aggregate after it is kept
-- and runs.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT coalesce(NULL::int, sum(1/b)) FROM u;
