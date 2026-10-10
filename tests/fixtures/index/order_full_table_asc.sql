-- Sort elision over the FULL table, no LIMIT: `ORDER BY seq ASC` on a NOT-NULL
-- UNIQUE-indexed column drops the Sort and walks the whole index ascending (an
-- unbounded ordered IndexRangeScan, scan limit None). The full ordered scan must
-- equal Postgres. `seq` is UNIQUE NOT NULL → total order → deterministic
-- positional compare.
-- @profile-lock
-- @setup
CREATE TABLE ord_full (id INT PRIMARY KEY, seq INT NOT NULL);
CREATE UNIQUE INDEX ord_full_seq ON ord_full(seq);
INSERT INTO ord_full VALUES
  (1, 50),
  (2, 10),
  (3, 40),
  (4, 20),
  (5, 30);
-- @test
SELECT id, seq FROM ord_full ORDER BY seq ASC;
