-- The CTE name shadows any same-named real table throughout the WITH RECURSIVE body,
-- so a reference to the name in the non-recursive (seed) term is an illegal
-- self-reference, not a read of the real table. Without the shadow pglet silently read
-- the base table's rows. The `@setup` creates a real `t` so the shadow is exercised.
-- @setup
CREATE TABLE t (n INT);
INSERT INTO t VALUES (100), (200);
-- @test
-- @expect-error: self-reference in the non-recursive term (42P19)
WITH RECURSIVE t(n) AS (
  SELECT n FROM t
  UNION ALL
  SELECT n + 1 FROM t WHERE n < 103
)
SELECT n FROM t ORDER BY n;
