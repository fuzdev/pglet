-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- Two keys, both junk and of different kinds (an aggregate and a grouped column), so
-- the Aggregate carries two junk columns at once and each has to get its own name.
-- @test
SELECT count(*) FROM gj GROUP BY k ORDER BY min(v), k;
