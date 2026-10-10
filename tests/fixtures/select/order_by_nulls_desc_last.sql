-- `ORDER BY v DESC NULLS LAST` — the non-default DESC placement (default DESC is
-- NULLS FIRST). NULLs sort after every non-NULL, values descending. Was silently
-- wrong: pglet put the NULLs first, ignoring the explicit clause.
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 30), (2, NULL), (3, 10), (4, 20), (5, NULL), (6, 20);
-- @test
SELECT id, v FROM t ORDER BY v DESC NULLS LAST, id ASC;
