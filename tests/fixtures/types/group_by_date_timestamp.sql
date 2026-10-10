-- GROUP BY over a mixed Date/Timestamp key: `COALESCE(d, ts)` keeps raw Date and
-- Timestamp variants in pglet, and a DATE must land in the SAME group as the equal
-- midnight-UTC TIMESTAMP (Postgres `date2timestamp`), matching DISTINCT. Regression
-- guard that the GROUP BY hash key agrees with the (Date,Timestamp) comparator arm.
-- @setup
CREATE TABLE m (id INT, d DATE, ts TIMESTAMP);
INSERT INTO m VALUES
  (1, '2024-01-01', NULL),
  (2, NULL, '2024-01-01 00:00:00'),
  (3, '2024-01-01', NULL),
  (4, '2024-01-02', NULL),
  (5, NULL, '2024-01-02 00:00:00'),
  (6, NULL, '2024-01-02 06:00:00');
-- @test
SELECT count(*) AS n FROM m GROUP BY COALESCE(d, ts) ORDER BY n;
