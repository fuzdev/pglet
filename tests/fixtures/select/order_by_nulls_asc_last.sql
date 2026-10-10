-- `ORDER BY v ASC NULLS LAST` — the ASC default made explicit; NULLs sort after
-- every non-NULL. Guards that the explicit clause and the default agree for ASC.
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 30), (2, NULL), (3, 10), (4, 20), (5, NULL), (6, 20);
-- @test
SELECT id, v FROM t ORDER BY v ASC NULLS LAST, id ASC;
