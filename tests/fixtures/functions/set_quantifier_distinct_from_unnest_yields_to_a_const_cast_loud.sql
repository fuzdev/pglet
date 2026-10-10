-- The FROM-clause route reads the same rule: 18.6 transforms the arrays before
-- `transformRangeFunction` looks at the decoration at all, so the cast beats the
-- suppressed expansion and the 42809 behind it. `refuse_from_unnest_quantifier` therefore
-- takes the array *expressions* rather than their count, and runs the transform first.
-- The `ALL` and unquantified spellings of this statement are 22P02 on both engines too.
-- 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type integer: "a"
SELECT * FROM unnest(DISTINCT ARRAY['a'::int]) AS u(x);
