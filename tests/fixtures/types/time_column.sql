-- A real TIME column: stored, rendered `HH:MM:SS[.ffffff]`, and chronologically
-- ordered (byte-for-byte vs Postgres, in both wire formats).
-- @setup
CREATE TABLE events (id INT, t TIME);
INSERT INTO events VALUES (1, '13:45:06'), (2, '01:02:03'), (3, '23:59:59.5');

-- @test
SELECT id, t FROM events ORDER BY t;
