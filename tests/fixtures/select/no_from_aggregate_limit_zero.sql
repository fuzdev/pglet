-- A no-FROM aggregate under `LIMIT 0`: no row.
-- @test
SELECT count(*) LIMIT 0;
