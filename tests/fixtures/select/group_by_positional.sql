-- A `GROUP BY` **ordinal** — the 1-based position of a select-list entry, Postgres's
-- `findTargetlistEntrySQL92` bare-`A_Const` branch. pglet resolved no GROUP BY
-- ordinals at all: `1` stayed an integer literal, grouped every row into one group,
-- and the ungrouped `x` in the select list then raised a spurious `42803`.
-- @setup
CREATE TABLE gbp (x INT, y INT);
INSERT INTO gbp VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT x AS gx, count(*) AS c FROM gbp GROUP BY 1 ORDER BY 1;
