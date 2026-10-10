-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- The canonical shape: the grouping column orders the result and is not in the
-- select list, so it rides the Aggregate as a junk column. The group sizes are all
-- different, so the row order is fully determined by `k` and no tie hides a
-- mis-ordering.
-- @test
SELECT count(*) FROM gj GROUP BY k ORDER BY k DESC;
