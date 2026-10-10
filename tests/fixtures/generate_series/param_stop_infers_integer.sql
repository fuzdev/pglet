-- A bare `$N` bound takes the resolved overload's type: `integer` beside an integer.
-- @param $1 int4 3
-- @test
SELECT * FROM generate_series(1, $1);
