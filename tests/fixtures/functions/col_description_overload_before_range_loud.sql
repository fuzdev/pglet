-- The overload is decided before the first argument's `oid` conversion runs, so an
-- out-of-range `int8` beside a `boolean` is `42883`, not the `22003` it would be beside
-- an `int4` (`col_description_oid_out_of_range_loud.sql`).
-- @test
-- @expect-error: col_description(bigint, boolean) does not exist
-- @expect-message
SELECT col_description(4294967296, true);
