-- A negative OFFSET fails loud (2201X) like Postgres, instead of silently treating
-- the clause as zero.
-- @expect-error: OFFSET must not be negative (2201X)
-- @test
SELECT 1 OFFSET -1;
