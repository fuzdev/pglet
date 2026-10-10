-- NUMERIC(precision, scale) rounds to scale on store (half-away-from-zero) and
-- pads the display scale, exactly as Postgres does.
-- @setup
CREATE TABLE money (id INT, amount NUMERIC(10, 2));
INSERT INTO money VALUES (1, 1.239), (2, 1.005), (3, 100), (4, -2.5), (5, 0.1);

-- @test
SELECT id, amount FROM money ORDER BY id;
