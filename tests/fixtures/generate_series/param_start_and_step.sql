-- Bare parameters in the start and step positions.
-- @param $1 int4 10
-- @param $2 int4 -4
-- @test
SELECT g FROM generate_series($1, 1, $2) g;
