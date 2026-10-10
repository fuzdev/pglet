-- Multi-key `ORDER BY` where each key carries its OWN direction and NULL placement:
-- `a ASC NULLS FIRST, b DESC NULLS LAST`. Both `a` and `b` have NULLs, so the
-- per-key placement (not a single global rule) decides each level; `id` is the
-- final deterministic tiebreaker.
-- @setup
CREATE TABLE t (id INT, a INT, b INT);
INSERT INTO t VALUES
  (1, 1, 5),
  (2, NULL, 9),
  (3, 1, NULL),
  (4, 2, 3),
  (5, NULL, NULL),
  (6, 1, 9),
  (7, 2, NULL);
-- @test
SELECT id, a, b FROM t ORDER BY a ASC NULLS FIRST, b DESC NULLS LAST, id ASC;
