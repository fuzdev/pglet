-- The set-op column fold reaches the array family through the same element-wise rule,
-- and got it wrong on both build profiles before: `ARRAY[1] UNION ARRAY[1::bigint]`
-- declared `integer[]` where Postgres declares `bigint[]`. Only the declared type
-- differs — the rendered values are identical — so the declared-type legs are the whole
-- assertion.
-- @test
SELECT x FROM (SELECT ARRAY[1] AS x UNION SELECT ARRAY[1::BIGINT]) q;
