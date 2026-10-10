-- A bound `date` parameter over the extended protocol (the spine's real driver
-- binding path): $1's type is inferred from the `d` column, decoded to a Date,
-- and the row matches — differential vs Postgres.
-- @setup
CREATE TABLE events (id INT, d DATE);
INSERT INTO events VALUES (1, '2024-06-15'), (2, '2024-01-02'), (3, '2024-12-31');

-- @param $1 date 2024-06-15
-- @test
SELECT id FROM events WHERE d = $1;
