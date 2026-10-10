-- Resolving an alias must not let a query group by an **aggregate**. Postgres's
-- `checkTargetlistEntrySQL92` rejects a matched target entry containing an aggregate
-- or window function under `EXPR_KIND_GROUP_BY`: `42803` "aggregate functions are not
-- allowed in GROUP BY".
--
-- This is the hazard the alias feature creates rather than one it inherits — before
-- aliases resolved at all, `GROUP BY c` was simply an unknown column (`42703`). Adding
-- the alias search without this check would accept grouping by `count(*)`.
-- @setup
CREATE TABLE gbaa (x INT, y INT);
INSERT INTO gbaa VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: aggregate functions are not allowed in GROUP BY (42803)
SELECT count(*) AS c FROM gbaa GROUP BY c;
