-- A real NUMERIC column: exact storage, `YYYY`-style decimal rendering, and
-- value ordering (byte-for-byte vs Postgres, both wire formats).
-- @setup
CREATE TABLE prices (id INT, amount NUMERIC);
INSERT INTO prices VALUES (1, 19.99), (2, 9.5), (3, 1234567890.12345), (4, -0.001), (5, 0);

-- @test
SELECT id, amount FROM prices ORDER BY amount;
