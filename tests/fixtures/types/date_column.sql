-- A real DATE column: stored, rendered `YYYY-MM-DD`, and chronologically ordered
-- (byte-for-byte vs Postgres, in both wire formats).
-- @setup
CREATE TABLE events (id INT, d DATE);
INSERT INTO events VALUES (1, '2024-06-15'), (2, '2024-01-02'), (3, '2024-12-31');

-- @test
SELECT id, d FROM events ORDER BY d;
