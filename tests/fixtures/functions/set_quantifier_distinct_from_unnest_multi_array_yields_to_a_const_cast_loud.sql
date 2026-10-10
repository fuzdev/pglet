-- The FROM route's own arity refusal is what the transform outranks here: with two arrays
-- and no cast the same statement is
-- `42883 function unnest(integer[], integer[]) does not exist` on both engines
-- (`set_quantifier_distinct_from_unnest_multi_array_loud.sql`), and adding the cast makes
-- it 22P02 on both. The pair fixes the transform's position at this route the way
-- `sequences/const_cast_beats_the_arity_loud.sql` fixes it at `build_call`'s.
-- 22P02 on both.
-- @test
-- @expect-error: invalid input syntax for type integer: "a"
SELECT * FROM unnest(DISTINCT ARRAY['a'::int], ARRAY['b'::int]) AS u(x, y);
