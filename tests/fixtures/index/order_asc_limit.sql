-- ORDER BY index pushdown (Sort elision): `ORDER BY seq ASC LIMIT 3` over a
-- NOT-NULL UNIQUE-indexed column lets the index supply the order directly, so the
-- planner drops the Sort and emits an ascending IndexRangeScan with the LIMIT as a
-- scan early-stop. `seq` is UNIQUE NOT NULL → the single-key order is total, so the
-- positional differential compare is deterministic on both sides.
-- @profile-lock
-- @setup
CREATE TABLE ord_asc (id INT PRIMARY KEY, seq INT NOT NULL);
CREATE UNIQUE INDEX ord_asc_seq ON ord_asc(seq);
INSERT INTO ord_asc VALUES
  (1, 50),
  (2, 10),
  (3, 40),
  (4, 20),
  (5, 30);
-- @test
SELECT id, seq FROM ord_asc ORDER BY seq ASC LIMIT 3;
