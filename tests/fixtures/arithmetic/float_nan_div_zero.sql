-- `NaN / 0` returns NaN, not a division-by-zero error — Postgres `float8div`'s
-- zero-divide check skips a NaN numerator (`!isnan(val1)`). A finite or infinite
-- numerator over zero is still a loud `22012` (see `float_div_by_zero.sql`).
-- @test
SELECT 'NaN'::float8 / 0.0 AS a;
