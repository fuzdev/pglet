-- An `IN` list beside an aggregate is not this rule.
-- @setup
CREATE TABLE al (g INT, n INT, i2 SMALLINT, i8 BIGINT, nu NUMERIC, f4 REAL, f8 FLOAT8, d DATE, ts TIMESTAMP, tz TIMESTAMPTZ, ti TIME, iv INTERVAL, ia INT[], by BYTEA, tx TEXT, vc VARCHAR(10), bo BOOL);
INSERT INTO al VALUES (1, 10, 1, 100, 1.5, 0.1, 0.1, '2024-01-01', '2024-01-01 10:00:00', '2024-01-01 10:00:00+00', '10:00:00', '1 day', '{1,2}', '\x01', 'a', 'a', true);
INSERT INTO al VALUES (1, 20, 2, 200, 2.5, 0.2, 0.2, '2024-06-01', '2024-06-01 10:00:00', '2024-06-01 10:00:00+00', '11:00:00', '2 days', '{3}', '\x02', 'b', 'b', false);
INSERT INTO al VALUES (2, 30, 3, 300, 3.5, 0.3, 0.3, '2022-01-01', '2022-01-01 10:00:00', '2022-01-01 10:00:00+00', '12:00:00', '3 days', '{1,2}', '\x03', 'c', 'c', true);
INSERT INTO al VALUES (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL);
CREATE TABLE al0 (g INT, n INT, nu NUMERIC, d DATE, ts TIMESTAMP);
-- @test
-- @expect-error: an IN list beside an aggregate stays loud
SELECT g, max(n) IN ('10', '30') FROM al GROUP BY g ORDER BY g;
