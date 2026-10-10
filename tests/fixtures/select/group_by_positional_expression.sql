-- An ordinal resolves to the target entry's **expression**, not to its output name.
-- Postgres binds `GROUP BY 1` to the `TargetEntry` itself, so the grouping key here is
-- `y - x` — which is also what makes the projection's whole-expression match against
-- the grouping key succeed. Resolving the ordinal to a by-name reference instead (what
-- `ORDER BY` / `DISTINCT ON` do, where a name is enough) would leave the projected
-- `y - x` unmatched and raise a spurious `42803`.
-- @setup
CREATE TABLE gbpe (x INT, y INT);
INSERT INTO gbpe VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT y - x AS diff, count(*) AS c FROM gbpe GROUP BY 1 ORDER BY 1;
