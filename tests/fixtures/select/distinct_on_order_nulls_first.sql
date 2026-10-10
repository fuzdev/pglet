-- `DISTINCT ON (k) … ORDER BY k, v NULLS FIRST` — the ON key's effective sort
-- carries the explicit NULL placement, so within each `k` group a NULL `v` sorts
-- FIRST and is the surviving row. This differs from the (buggy) direction default
-- (ASC → NULLS LAST, which would keep the non-NULL `v`), so it guards that the
-- placement flows through DISTINCT ON's merged sort. Outer order is the same key.
-- @setup
CREATE TABLE t (k INT, v INT);
INSERT INTO t VALUES (1, NULL), (1, 5), (2, 3), (2, NULL), (3, 7), (3, 8);
-- @test
SELECT DISTINCT ON (k) k, v FROM t ORDER BY k, v NULLS FIRST;
