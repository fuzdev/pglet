-- The `24:00:00` endpoint is a valid TIME (Postgres accepts it as the day's upper
-- bound); it round-trips and renders as `24:00:00`, in both wire formats.
-- @test
SELECT '24:00:00'::time AS endpoint;
