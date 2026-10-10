-- The other half of the inverted-precedence rule (see
-- `group_by_alias_vs_column_precedence.sql`): in **ORDER BY** the output alias wins
-- over the same-named FROM column, because `findTargetlistEntrySQL92` runs its
-- FROM-column precheck for `GROUP BY` only.
--
-- `a` is both a column of `pgc` and the alias of the projected `b`. The ORDER BY key
-- `a` binds to the alias — the projected `b` — so the rows come back ordered by `b`
-- descending. Binding it to the FROM column `pgc.a` instead would be an error (`a` is
-- not grouped), so this fixture also proves the GROUP BY rule was not applied here.
-- @setup
CREATE TABLE pgc (a INT PRIMARY KEY, b INT);
INSERT INTO pgc VALUES (1, 10), (2, 10), (3, 20);
-- @test
SELECT b AS a, count(*) AS c FROM pgc GROUP BY b ORDER BY a DESC;
