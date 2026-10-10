-- An aggregate under a temporal function: `date_trunc`, `extract`, `date_part`, and a
-- `timestamptz` rendered through `concat` and `||`, which must keep its `+00` offset from
-- the aggregate's declared type.
-- @setup
CREATE TABLE tt (t TIMESTAMP, d DATE, tz TIMESTAMPTZ);
INSERT INTO tt VALUES ('2024-01-02 03:04:05', '2024-01-02', '2024-01-02 03:04:05+00'),
  ('2025-06-07 08:09:10', '2025-06-07', '2025-06-07 08:09:10+00');
-- @test
SELECT date_trunc('day', max(t)) AS dt, extract(year FROM max(t)) AS ey,
  date_part('month', min(d)) AS dp, date_trunc('day', max(tz)) AS dtz,
  concat('c=', max(tz)) AS ctz, 'p=' || min(tz) AS ptz
FROM tt;
