-- A `CHECK` reads the assembled new row, `(11, 10)`, which satisfies it; the in-place
-- reading `(11, 11)` would violate it. pglet refused `23514`.
-- @setup
CREATE TABLE socp (a INT, b INT, CHECK (a <> b));
INSERT INTO socp VALUES (10, 0);

-- @test
UPDATE socp SET a = a + 1, b = a RETURNING a, b;
