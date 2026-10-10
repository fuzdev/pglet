-- A negative LIMIT fails loud (2201W) like Postgres, instead of silently treating
-- the clause as unbounded and returning every row.
-- @expect-error: LIMIT must not be negative (2201W)
-- @test
SELECT 1 LIMIT -1;
