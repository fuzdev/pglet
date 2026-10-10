-- In the `(int4, int4)` overload each bare parameter is typed `integer`.
-- @param $1 int4 1
-- @param $2 int4 2
-- @test
SELECT pg_advisory_xact_lock($1, $2) AS k;
