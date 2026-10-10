-- Integer division by zero errors (Postgres 22012), instead of the old
-- `f64::NAN as i64 == 0` that silently returned 0.
-- @test
-- @expect-error: division by zero
SELECT 1 / 0;
