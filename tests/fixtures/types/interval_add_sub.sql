-- `interval ± interval` is field-wise with no auto-justify (Postgres keeps
-- `1 day - 25 hours` as `1 day -25:00:00`); unary `-interval` negates every field.
-- @test
SELECT INTERVAL '1 day' + INTERVAL '2 hours'       AS add,
       INTERVAL '1 day' - INTERVAL '25 hours'      AS sub_no_justify,
       INTERVAL '1 year 2 mons' + INTERVAL '3 mons' AS add_months,
       -INTERVAL '1 day 2 hours'                   AS negate;
