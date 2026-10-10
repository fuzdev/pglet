-- The explicit `-> $1::int` spelling: the cast pins the parameter to `integer`,
-- which is the `(jsonb, integer)` signature, so the bound value indexes the array.
-- The counterpart to `jsonb_arrow_param_is_text`.
-- @param $1 int4 1
-- @test
SELECT '[10,20,30]'::jsonb -> $1::int AS arr_by_index;
