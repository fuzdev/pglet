-- Two select-list entries carrying the same output alias with **different**
-- expressions make a `GROUP BY` key naming that alias ambiguous: `42702` "GROUP BY
-- \"a\" is ambiguous". Postgres's `findTargetlistEntrySQL92` does not stop at the first
-- alias match — it scans the whole target list and raises `ERRCODE_AMBIGUOUS_COLUMN`
-- on a second entry whose expression is not `equal()` to the first.
--
-- pglet's alias search was a first-match `position()`, so this silently grouped by `x`
-- and returned rows Postgres refuses. Counting matches is not enough either: the
-- duplicate is legal when the entries denote the same expression, which
-- `group_by_alias_duplicate_same_expr.sql` pins from the other side.
-- @setup
CREATE TABLE gbamb (x INT, y INT);
INSERT INTO gbamb VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: GROUP BY "a" is ambiguous (42702)
SELECT x AS a, count(*) AS a FROM gbamb GROUP BY a ORDER BY 1;
