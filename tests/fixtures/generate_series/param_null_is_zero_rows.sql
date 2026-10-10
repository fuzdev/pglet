-- A NULL bound is zero rows.
-- @param $1 int4 NULL
-- @test
SELECT * FROM generate_series(1, $1);
