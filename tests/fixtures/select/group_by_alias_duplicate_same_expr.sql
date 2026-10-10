-- The accepting half of the `GROUP BY` alias-ambiguity rule: a duplicated output alias
-- is **not** ambiguous when both entries denote the same expression. Postgres permits
-- the second match when `equal(target_result->expr, tle->expr)`, so `SELECT x AS a,
-- x AS a … GROUP BY a` groups by `x` and returns a row per distinct `x`.
--
-- This is what forces the ambiguity check to compare *expressions* rather than count
-- how many entries carry the name; a count-based check would reject this query.
-- `group_by_alias_ambiguous.sql` pins the rejecting half.
--
-- Wrapped in a subquery because the harness keys result rows by column name, and the
-- query whose behavior is under test necessarily projects two columns both named `a`.
-- The aggregate over it observes the grouped row count, which is the thing at stake:
-- 2 (grouped by `x`) rather than 3 (ungrouped) or an error.
-- @setup
CREATE TABLE gbdup (x INT, y INT);
INSERT INTO gbdup VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT count(*) AS grouped_rows FROM (SELECT x AS a, x AS a FROM gbdup GROUP BY a) s;
