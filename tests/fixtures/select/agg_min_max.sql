-- @setup
CREATE TABLE temps (id INT, temperature INT);
INSERT INTO temps VALUES (1, 20), (2, 35), (3, 15), (4, 28);

-- @test
SELECT MIN(temperature), MAX(temperature) FROM temps;
