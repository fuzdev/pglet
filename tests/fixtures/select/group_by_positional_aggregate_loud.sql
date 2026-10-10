-- The aggregate check applies to the **ordinal** spelling too, not just the alias one:
-- `GROUP BY 1` names the sole target entry, `count(*)`, so it is `42803` "aggregate
-- functions are not allowed in GROUP BY". Postgres runs `checkTargetlistEntrySQL92` on
-- whatever entry the SQL92 search matched, however it was addressed.
--
-- The select list is deliberately the aggregate **alone**. With an ungrouped column
-- alongside it (`SELECT x AS gx, count(*) AS c … GROUP BY 2`) the ordinary grouping
-- rule also raises `42803`, so the fixture would stay green with the aggregate check
-- removed and lock nothing.
-- @setup
CREATE TABLE gbpa (x INT, y INT);
INSERT INTO gbpa VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: aggregate functions are not allowed in GROUP BY (42803)
SELECT count(*) AS c FROM gbpa GROUP BY 1;
