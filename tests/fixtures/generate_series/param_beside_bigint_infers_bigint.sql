-- Beside a `bigint`, a bare `$N` is `bigint`, and so is the column.
-- @param $1 int8 3
-- @test
SELECT * FROM generate_series(1::bigint, $1);
