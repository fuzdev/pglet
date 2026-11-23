-- A bare `$1` in the `(bigint)` overload is typed `bigint`, as 18.6 types it.
-- @param $1 int8 3853314791062309107
-- @test
SELECT pg_advisory_xact_lock_shared($1) AS k;
