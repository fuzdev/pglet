-- The ordinal spelling of `group_by_window_alias_loud.sql`: `checkTargetlistEntrySQL92`
-- runs on whichever target entry matched, so reaching a window function by position is
-- the same `42P20` as reaching it by output alias.
-- @setup
CREATE TABLE gbwp (x INT, y INT);
INSERT INTO gbwp VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: window functions are not allowed in GROUP BY (42P20)
SELECT row_number() OVER () AS w FROM gbwp GROUP BY 1;
