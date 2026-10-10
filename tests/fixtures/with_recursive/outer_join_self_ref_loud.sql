-- A self-reference on the nullable (right) side of a LEFT JOIN is illegal. Under plain
-- UNION this can terminate with rows Postgres rejects (silently wrong without the
-- placement check); the `@setup` rows make that would-be result concrete.
-- @setup
CREATE TABLE base (x INT);
INSERT INTO base VALUES (1), (2), (3), (4);
-- @test
-- @expect-error: self-reference on the nullable side of an outer join (42P19)
WITH RECURSIVE t(n) AS (
  SELECT 1
  UNION
  SELECT base.x FROM base LEFT JOIN t ON base.x = t.n WHERE base.x < 5
)
SELECT n FROM t ORDER BY n;
