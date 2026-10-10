-- The `GROUP BY` half of the same rule: `-1` folds to a constant, so it is a position and
-- `42P10 GROUP BY position -1 is not in select list`. Without the fold the key is an
-- ordinary constant grouping expression, which leaves the projected column ungrouped and
-- answers `42803` here instead — a different error on a statement 18.6 refuses for a
-- different reason, so neither engine's message identifies the other's.
-- @setup
CREATE TABLE gbnp (x INT, y INT);
INSERT INTO gbnp VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: GROUP BY position -1 is not in select list (42P10)
SELECT x FROM gbnp GROUP BY -1;
