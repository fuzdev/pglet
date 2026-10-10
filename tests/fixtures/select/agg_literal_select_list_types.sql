-- The literal beside an aggregate takes each result type the aggregate is certain of: smallint, bigint,
-- numeric, real (inexact), double precision, timestamptz, time, interval, an array, bytea.
-- @setup
CREATE TABLE al (g INT, n INT, i2 SMALLINT, i8 BIGINT, nu NUMERIC, f4 REAL, f8 FLOAT8, d DATE, ts TIMESTAMP, tz TIMESTAMPTZ, ti TIME, iv INTERVAL, ia INT[], by BYTEA, tx TEXT, vc VARCHAR(10), bo BOOL);
INSERT INTO al VALUES (1, 10, 1, 100, 1.5, 0.1, 0.1, '2024-01-01', '2024-01-01 10:00:00', '2024-01-01 10:00:00+00', '10:00:00', '1 day', '{1,2}', '\x01', 'a', 'a', true);
INSERT INTO al VALUES (1, 20, 2, 200, 2.5, 0.2, 0.2, '2024-06-01', '2024-06-01 10:00:00', '2024-06-01 10:00:00+00', '11:00:00', '2 days', '{3}', '\x02', 'b', 'b', false);
INSERT INTO al VALUES (2, 30, 3, 300, 3.5, 0.3, 0.3, '2022-01-01', '2022-01-01 10:00:00', '2022-01-01 10:00:00+00', '12:00:00', '3 days', '{1,2}', '\x03', 'c', 'c', true);
INSERT INTO al VALUES (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
CREATE TABLE al0 (g INT, n INT, nu NUMERIC, d DATE, ts TIMESTAMP);
-- @test
SELECT g, max(i2) = '2' AS i2_max, sum(i2) = '3' AS i2_sum, sum(i8) = '300' AS i8_sum, avg(i8) > '149.5' AS i8_avg, sum(f4) = '0.3' AS f4_sum, max(f4) = '0.2' AS f4_max, avg(f8) < '0.2' AS f8_avg, max(tz) = '2024-06-01 10:00:00+00' AS tz_max, max(ti) >= '11:00' AS ti_max, sum(iv) = '3 days' AS iv_sum, avg(iv) = '1 day 12:00:00' AS iv_avg, max(ia) = '{3}' AS ia_max, min(by) = '\x01' AS by_min FROM al GROUP BY g ORDER BY g;
