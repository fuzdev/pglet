-- The flagship keyset shape: `WHERE seq > $1 ORDER BY seq ASC LIMIT $2` (bound
-- params, the spine's real binding path). The range WHERE builds an IndexRangeScan
-- with a lower bound; the ORDER BY pushdown sets it ascending and the LIMIT pushes
-- a scan early-stop — ONE node, no Sort. Mirrors the spine's `fact_queries` /
-- audit `… ORDER BY seq DESC LIMIT` pagination. `seq` is UNIQUE NOT NULL → total
-- order → deterministic positional compare.
-- @profile-lock
-- @setup
CREATE TABLE ord_range (id INT PRIMARY KEY, seq INT NOT NULL);
CREATE UNIQUE INDEX ord_range_seq ON ord_range(seq);
INSERT INTO ord_range VALUES
  (1, 10),
  (2, 20),
  (3, 30),
  (4, 40),
  (5, 50),
  (6, 60);
-- @param $1 int4 20
-- @param $2 int8 3
-- @test
SELECT id, seq FROM ord_range WHERE seq > $1 ORDER BY seq ASC LIMIT $2;
