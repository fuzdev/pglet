-- `SELECT *` over an aggregated query, grouping by the columns spelled out. The sole
-- bare `*` is deliberately left unexpanded for the executor's projection fast path, but
-- the `Aggregate` path has no such fast path, so this was rejected as `42703 column "*"
-- does not exist` where Postgres returns the rows. Independent of the ordinal rules —
-- no `GROUP BY` position appears here.
-- @setup
CREATE TABLE gbsc (x INT, y INT);
INSERT INTO gbsc VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT * FROM gbsc GROUP BY x, y ORDER BY 1, 2;
