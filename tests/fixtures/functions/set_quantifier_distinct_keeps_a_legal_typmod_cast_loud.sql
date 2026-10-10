-- A typmod'd constant cast whose input conversion **succeeds** must reach the quantifier's
-- own verdict rather than an error of the transform's making: `'abcdef'` is a legal
-- `varchar`, the modifier truncates it in the explicit context, and what is wrong with the
-- statement is the `DISTINCT`. This is the control that separates "validate the input
-- conversion" from "apply the modifier" — the accepting spelling of the same cast is
-- `set_quantifier_all_typmod_cast_truncates.sql`. 42809 on both.
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function (42809)
SELECT upper(DISTINCT 'abcdef'::varchar(2));
