-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- An aggregate the select list drops is the other kind of junk entry. `sum(v)`
-- ranks the groups the same way `k` does here, and the select list carries neither,
-- so the Sort has nothing to read without it.
-- @test
SELECT k FROM gj GROUP BY k ORDER BY sum(v) DESC;
