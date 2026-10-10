-- The `DISTINCT ON` half: a folded `-1` is a position there too, so
-- `42P10 DISTINCT ON position -1 is not in select list`. Unfolded it was an ordinary
-- constant `ON` key — every row in one dedup group, so the statement answered a single
-- arbitrary row where 18.6 refuses.
-- @setup
CREATE TABLE donp (x INT, y INT);
INSERT INTO donp VALUES (1, 10), (1, 20), (2, 30);
-- @test
-- @expect-error: DISTINCT ON position -1 is not in select list (42P10)
SELECT DISTINCT ON (-1) x FROM donp;
