-- The accepting polarity of the typmod'd constant cast, and the reason the transform
-- validates the input conversion and nothing else: a `varchar(n)` over-run is not an
-- error in the explicit-cast context, it truncates. So the argument transform must let
-- this statement through with the value 18.6 gives it, `AB`, rather than raising — which
-- is what a transform that applied the modifier in the *column-store* context would do
-- (22001), and what a transform that applied it at all would risk. Its refusing twin is
-- `set_quantifier_distinct_keeps_a_legal_typmod_cast_loud.sql`, where the same cast is
-- legal and the quantifier is what fails.
-- @test
SELECT upper(ALL 'abcdef'::varchar(2)) AS u, 'abcdef'::varchar(2) AS v;
