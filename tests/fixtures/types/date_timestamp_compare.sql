-- A DATE compared with a TIMESTAMP promotes the date to midnight UTC (Postgres
-- `date2timestamp`), so it is EQUAL at the exact midnight instant and ordered
-- correctly just around it. Regression guard for the missing `(Date, Timestamp)`
-- cross arm (which fell through to a text fallback, wrong at/around midnight).
-- @test
SELECT
  ('2024-01-01'::date =  '2024-01-01 00:00:00'::timestamp) AS eq_mid,
  ('2024-01-01'::date <  '2024-01-01 00:00:01'::timestamp) AS lt_after,
  ('2024-01-01'::date >  '2023-12-31 23:59:59'::timestamp) AS gt_before,
  ('2024-01-01'::date <  '2024-01-01 00:00:00'::timestamp) AS lt_mid,
  ('2024-01-01 00:00:01'::timestamp > '2024-01-01'::date)  AS ts_gt_date
;
