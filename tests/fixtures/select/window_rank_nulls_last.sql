-- A window `OVER (ORDER BY v DESC NULLS LAST)` — the non-default DESC placement.
-- RANK peers (equal `v`) share a rank; the two NULL rows are peers that sort LAST
-- (not first, the DESC default), so they take the trailing rank. Was silently
-- wrong: the window ORDER BY dropped `NULLS FIRST/LAST` too. Outer `ORDER BY r, id`
-- makes the output deterministic (RANK is well-defined for peers).
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 20), (2, NULL), (3, 10), (4, 20), (5, NULL);
-- @test
SELECT id, v, RANK() OVER (ORDER BY v DESC NULLS LAST) AS r
FROM t
ORDER BY r, id;
