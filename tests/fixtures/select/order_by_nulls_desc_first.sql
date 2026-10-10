-- `ORDER BY v DESC NULLS FIRST` — the DESC default made explicit; NULLs sort
-- before every non-NULL, values descending.
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 30), (2, NULL), (3, 10), (4, 20), (5, NULL), (6, 20);
-- @test
SELECT id, v FROM t ORDER BY v DESC NULLS FIRST, id ASC;
