-- A cyclic graph reachability: plain UNION dedups whole rows against everything seen
-- so far, so the 1->2->3->1 cycle terminates (UNION ALL would loop forever). This
-- exercises the dedup termination path.
-- @setup
CREATE TABLE edge (src INT, dst INT);
INSERT INTO edge VALUES (1, 2), (2, 3), (3, 1);
-- @test
WITH RECURSIVE reach(n) AS (
  SELECT 1
  UNION
  SELECT e.dst FROM edge e JOIN reach r ON e.src = r.n
)
SELECT n FROM reach ORDER BY n;
