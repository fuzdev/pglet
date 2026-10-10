-- A `GROUP BY` ordinal past the select-list length is `42P10` "GROUP BY position 99 is
-- not in select list" — the same range rule `ORDER BY` / `DISTINCT ON` already
-- enforced, now reached from GROUP BY. pglet accepted this silently, grouping every
-- row into one group. `GROUP BY 0` is the same error (positions are 1-based) and is
-- exercised by `group_by_position_zero.sql`.
-- @setup
CREATE TABLE gboor (x INT, y INT);
INSERT INTO gboor VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: GROUP BY position out of range (42P10)
SELECT x AS gx, count(*) AS c FROM gboor GROUP BY 99;
