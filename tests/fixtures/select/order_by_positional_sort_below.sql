-- A positional `ORDER BY n` names the Nth OUTPUT column even when the Sort sits
-- BELOW the projection (forced here by the dropped base column `c` in the sort
-- key). `ORDER BY c, 1` orders by `c`, then by output column 1 (`b`) — not by the
-- base column at index 1.
-- @setup
CREATE TABLE t (a INT, b INT, c INT);
INSERT INTO t VALUES (100, 5, 2), (200, 9, 1), (300, 7, 1);

-- @test
SELECT b, a FROM t ORDER BY c, 1;
