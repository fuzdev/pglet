-- The other half of the interleave pair (see
-- `select/no_from_order_by_bogus_key_before_a_bad_position_loud.sql`): the first key's
-- out-of-range ordinal wins, `42P10` on both engines.
-- @expect-error: ORDER BY position 9 is not in select list (42P10)
-- @test
SELECT 1 AS q ORDER BY 9, zzz;
