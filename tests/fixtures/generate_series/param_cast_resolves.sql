-- A cast parameter is typed as written.
-- @param $1 int8 2
-- @test
SELECT * FROM generate_series(1, $1::int8);
