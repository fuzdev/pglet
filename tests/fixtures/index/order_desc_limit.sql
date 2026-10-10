-- ORDER BY index pushdown, descending path: `ORDER BY seq DESC LIMIT 3` over a
-- NOT-NULL UNIQUE-indexed column emits a DESCENDING IndexRangeScan (Sort dropped),
-- the index walked back-to-front. `seq` is UNIQUE NOT NULL → total order →
-- deterministic positional compare against Postgres.
-- @profile-lock
-- @setup
CREATE TABLE ord_desc (id INT PRIMARY KEY, seq INT NOT NULL);
CREATE UNIQUE INDEX ord_desc_seq ON ord_desc(seq);
INSERT INTO ord_desc VALUES
  (1, 50),
  (2, 10),
  (3, 40),
  (4, 20),
  (5, 30);
-- @test
SELECT id, seq FROM ord_desc ORDER BY seq DESC LIMIT 3;
