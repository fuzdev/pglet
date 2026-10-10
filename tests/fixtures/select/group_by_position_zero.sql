-- `GROUP BY 0` is out of range: positions are 1-based, so Postgres reports `42P10`
-- "GROUP BY position 0 is not in select list" exactly as it does for an ordinal past
-- the end. Split out from `group_by_position_out_of_range.sql`, which asserted this
-- case in prose while only exercising `99`.
-- @setup
CREATE TABLE gbpz (x INT, y INT);
INSERT INTO gbpz VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: GROUP BY position 0 is not in select list (42P10)
SELECT x AS gx, count(*) AS c FROM gbpz GROUP BY 0;
