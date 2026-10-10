-- A schema-qualified name is a relation, never a CTE, and neither is its schema part:
-- `public.b` reads the base table, so `a` depends on no CTE — not on `b`, and not on the
-- CTE this list names `public` — and there is no cycle.
-- @setup
CREATE TABLE b (x INT);
INSERT INTO b VALUES (1), (2);
-- @test
WITH RECURSIVE a AS (SELECT * FROM public.b),
  public AS (SELECT * FROM a),
  b AS (SELECT * FROM a)
SELECT x FROM public ORDER BY x;
