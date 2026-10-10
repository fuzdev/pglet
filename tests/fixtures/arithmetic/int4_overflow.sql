-- Int4 arithmetic overflow errors ("integer out of range", Postgres 22003)
-- rather than saturating at i32::MAX the way `f64 as i32` did.
-- @test
-- @expect-error: integer out of range
SELECT 2147483647 + 1;
