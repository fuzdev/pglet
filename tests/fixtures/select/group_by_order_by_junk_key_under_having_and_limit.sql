-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- The junk trim has to sit above the Sort and below the Limit: trimming under the
-- Sort loses the key, and the Limit reads no column at all. `HAVING` is on the same
-- statement because it is the other clause evaluated inside the Aggregate.
-- @test
SELECT count(*) FROM gj GROUP BY k HAVING count(*) > 1 ORDER BY k DESC LIMIT 1;
