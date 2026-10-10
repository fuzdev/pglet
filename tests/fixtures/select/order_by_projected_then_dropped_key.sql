-- @setup
CREATE TABLE sd (x INT, a INT, b INT);
INSERT INTO sd VALUES (1, 20, 300), (1, 10, 100), (2, 40, 200), (3, 30, 400);

-- The most ordinary shape there is: order by a projected column, then tie-break on
-- one the select list drops. The two kinds of key mix in a single ORDER BY, and the
-- Sort must go *below* the projection to still see `a` — so a placement rule that
-- refuses sort-below whenever any key can be read from the output rejects this
-- outright, and the dropped `a` then fails `42703` above the projection.
-- @test
SELECT x FROM sd ORDER BY x, a;
