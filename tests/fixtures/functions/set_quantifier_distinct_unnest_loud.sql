-- The single-argument arm (`single_arg`, shared with `lower`), in the select-list
-- position. 42809 on both; pglet answered two rows. The **FROM-clause** spelling is a
-- different parser route reaching the same code by a different rule — see
-- `set_quantifier_distinct_from_unnest_loud.sql` and its multi-array sibling, where the
-- verdict is `transformRangeFunction`'s suppressed expansion rather than this arm.
-- @test
-- @expect-error: DISTINCT specified, but unnest is not an aggregate function
SELECT unnest(DISTINCT ARRAY[1, 2]);
