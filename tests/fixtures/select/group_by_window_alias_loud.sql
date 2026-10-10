-- A `GROUP BY` key resolving to a target entry that is a **window function** is
-- `42P20` "window functions are not allowed in GROUP BY", not the aggregate's `42803`.
-- Postgres's `checkTargetlistEntrySQL92` tests the two separately and in that order,
-- with `ERRCODE_GROUPING_ERROR` for aggregates and `ERRCODE_WINDOWING_ERROR` for
-- windows.
--
-- pglet collapsed both into the aggregate error, reporting a window function as
-- "aggregate functions are not allowed in GROUP BY" under `42803`.
-- @setup
CREATE TABLE gbwa (x INT, y INT);
INSERT INTO gbwa VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: window functions are not allowed in GROUP BY (42P20)
SELECT row_number() OVER () AS w FROM gbwa GROUP BY w;
