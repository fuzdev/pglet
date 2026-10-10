-- A NULLABLE, single-column-indexed column ordered with an explicit `NULLS FIRST`.
-- The Sort-elision planner must NOT push this into an ordered index scan: a plain
-- B-tree does not index a NULL-keyed row, so an ordered scan would omit the NULL
-- rows entirely. The retained Sort honors the placement. Runs under both index-accel
-- profiles (accel ON by default, and `PGLET_NO_INDEX_ACCEL=1`); both must equal
-- Postgres — the NULL rows are present and sorted first.
-- @setup
CREATE TABLE t (id INT, v INT);
CREATE INDEX t_v_idx ON t (v);
INSERT INTO t VALUES (1, 30), (2, NULL), (3, 10), (4, 20), (5, NULL), (6, 20);
-- @test
SELECT id, v FROM t ORDER BY v ASC NULLS FIRST, id ASC;
