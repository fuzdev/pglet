-- `ORDER BY v ASC NULLS FIRST` — the non-default ASC placement (default ASC is
-- NULLS LAST). NULLs sort before every non-NULL; `id` is a deterministic tiebreaker
-- (it also orders the two NULL rows among themselves). Was silently wrong: pglet
-- put the NULLs last, ignoring the explicit clause.
-- @setup
CREATE TABLE t (id INT, v INT);
INSERT INTO t VALUES (1, 30), (2, NULL), (3, 10), (4, 20), (5, NULL), (6, 20);
-- @test
SELECT id, v FROM t ORDER BY v ASC NULLS FIRST, id ASC;
