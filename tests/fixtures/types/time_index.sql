-- A TIME primary key exercises the order-preserving index key: a range predicate
-- and ORDER BY both rely on the big-endian key's chronological order.
-- Differential vs Postgres.
-- @setup
CREATE TABLE shifts (t TIME PRIMARY KEY, label TEXT);
INSERT INTO shifts VALUES
  ('08:00:00', 'morning'),
  ('12:30:00', 'noon'),
  ('17:45:00', 'evening'),
  ('23:00:00', 'night');

-- @test
SELECT t, label FROM shifts WHERE t >= '10:00:00' AND t < '20:00:00' ORDER BY t;
