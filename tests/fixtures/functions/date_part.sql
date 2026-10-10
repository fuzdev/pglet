-- date_part is the float8-returning twin of EXTRACT (same field semantics). The text
-- render differs — a float8 `13`, not EXTRACT's numeric `13`. date_part('hour', date)
-- casts the date to timestamp (a SQL wrapper), so a time field succeeds (= 0) where
-- EXTRACT(hour FROM date) would fail (extract_wrong_source.sql).
-- @test
SELECT
  date_part('hour',    '2024-03-15 13:45:06'::timestamp) AS h,
  date_part('minute',  '2024-03-15 13:45:06'::timestamp) AS mi,
  date_part('year',    '2024-03-15'::date)               AS y,
  date_part('dow',     '2024-03-15'::date)               AS dow,
  date_part('epoch',   INTERVAL '5 days 3 hours')        AS iv_epoch,
  date_part('hour',    '2024-03-15'::date)               AS date_hour;
