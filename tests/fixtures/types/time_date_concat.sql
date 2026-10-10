-- `date + time → timestamp` (`time + date` commutative): UTC midnight of the date
-- plus the time-of-day, a `timestamp` without a time zone. Differential vs Postgres.
-- @test
SELECT '2024-06-15'::date + '13:30:00'::time AS date_plus_time,
       '09:15:00'::time + '2024-06-15'::date AS time_plus_date;
