-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- An aggregate nested inside a larger key: the key is not itself an aggregate, so
-- the walk reaches one and the *whole* key becomes junk. Evaluating any part of it
-- above the Aggregate is impossible.
-- @test
SELECT k FROM gj GROUP BY k ORDER BY sum(v) / count(*) DESC;
