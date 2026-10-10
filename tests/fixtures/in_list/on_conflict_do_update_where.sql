-- `ON CONFLICT … DO UPDATE … WHERE`: the list over the target's and the excluded row's column.
-- @setup
CREATE TABLE inl_k (id INT PRIMARY KEY, x REAL);
INSERT INTO inl_k VALUES (1, 0.1), (2, 1.5), (3, 7);
-- @test
INSERT INTO inl_k VALUES (1, 0.1), (2, 0.1), (3, 0.1)
ON CONFLICT (id) DO UPDATE SET x = 9 WHERE inl_k.x IN (0.1, 1.5) AND excluded.x IN (0.1, 7) RETURNING id, x;
