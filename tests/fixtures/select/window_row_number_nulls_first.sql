-- A window `OVER (ORDER BY v ASC NULLS FIRST, id)` — the non-default ASC placement,
-- with `id` a tiebreaker so ROW_NUMBER is a total, deterministic order. The two NULL
-- rows sort FIRST (positions 1 and 2), not last (the ASC default). Outer `ORDER BY
-- rn` reproduces the window order.
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 20), (2, NULL), (3, 10), (4, 20), (5, NULL);
-- @test
SELECT id, v, ROW_NUMBER() OVER (ORDER BY v ASC NULLS FIRST, id) AS rn
FROM t
ORDER BY rn;
