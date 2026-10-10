-- Temporal items of another type than the left operand: a `date` beside a `timestamp`
-- column, a `timestamp` beside a `timestamptz` one, a `time` beside an `interval` one.
-- @setup
CREATE TABLE inl_t (id INT, dt DATE, ts TIMESTAMP, tz TIMESTAMPTZ, iv INTERVAL);
INSERT INTO inl_t VALUES
  (1, '2020-01-01', '2020-01-01 00:00:00', '2020-01-01 00:00:00+00', '1 hour'),
  (2, '2020-01-02', '2020-01-02 10:00:00', '2020-01-02 10:00:00+00', '2 hours'),
  (3, '2020-01-03', '2020-01-03 00:00:00', '2020-01-03 00:00:00+00', '3 hours');
-- @test
SELECT id,
       ts IN ('2020-01-01'::date, '2020-01-03'::date) AS ts_dates,
       tz IN ('2020-01-01'::date, '2020-01-02 10:00:00'::timestamp) AS tz_mixed,
       iv IN ('1 hour', '02:00'::time) AS iv_time
FROM inl_t ORDER BY id;
