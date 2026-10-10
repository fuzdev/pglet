-- `jsonb -> $1` resolves the parameter to **text**, not integer: `->` is
-- overloaded on `(jsonb, text)` and `(jsonb, integer)`, and an unknown-typed
-- parameter takes the preferred type of the string category. So a bound `1` is the
-- object key `"1"`, and against an array it is NULL — the same answer Postgres
-- gives (`PREPARE … SELECT '[…]'::jsonb -> $1` reports `parameter_types = {text}`).
-- An array index needs the explicit `-> $1::int` spelling on both engines.
-- @param $1 text 1
-- @test
SELECT
  '[10,20,30]'::jsonb -> $1        AS arr_by_text_key,
  '{"1":"v"}'::jsonb ->> $1        AS obj_by_text_key;
