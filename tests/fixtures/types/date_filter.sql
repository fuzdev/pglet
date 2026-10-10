-- DATE equality and range predicates against an ISO literal — the comparison
-- coerces the literal to `date`, matching Postgres (no rows dropped).
-- @setup
CREATE TABLE events (id INT, d DATE);
INSERT INTO events VALUES (1, '2024-06-15'), (2, '2024-01-02'), (3, '2024-12-31');

-- @test
SELECT id FROM events WHERE d > '2024-06-01' AND d <> '2024-12-31' ORDER BY id;
