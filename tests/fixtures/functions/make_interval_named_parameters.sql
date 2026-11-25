-- A bare `$N` takes its position's type: `integer` for the fields, `double precision` for
-- `secs`.
-- @param $1 int4 3
-- @param $2 float8 1.5
-- @test
SELECT make_interval(hours => $1) AS h, make_interval(days => $1, secs => $2) AS ds;
