-- Postgres's documented exception to "the structures must match": **an array may
-- contain a primitive value**, so `'["a","b"]' @> '"a"'` is true. It falls out of
-- the storage model — a top-level scalar is a one-element *raw scalar* pseudo-array,
-- so this is the ordinary array rule.
--
-- The exception is one-directional and **top-level only**, which is the part an
-- implementation gets wrong:
--
-- - a raw scalar does not contain an array (`'"a"' @> '["a"]'` is false — a raw
--   scalar may contain another raw scalar, never a real array);
-- - it does not apply inside an object, where a value's type must match exactly, so
--   `'{"a":[1,2]}' @> '{"a":1}'` is **false** even though the array `[1,2]` would
--   contain the scalar `1` at the top level.
-- @setup
CREATE TABLE jcrs (id INT, d JSONB);
INSERT INTO jcrs VALUES (1, '["a","b"]'), (2, '"a"'), (3, '[1,2,3]');
-- @test
SELECT
  ('["a","b"]'::jsonb @> '"a"'::jsonb) AS array_contains_primitive,
  ('[1,2,3]'::jsonb @> '3'::jsonb) AS array_contains_number,
  ('"a"'::jsonb @> '["a"]'::jsonb) AS scalar_does_not_contain_array,
  ('"a"'::jsonb @> '"a"'::jsonb) AS scalar_contains_itself,
  ('{"a":[1,2]}'::jsonb @> '{"a":1}'::jsonb) AS no_exception_when_nested,
  ('{"a":1}'::jsonb @> '"a"'::jsonb) AS object_never_contains_scalar,
  (SELECT count(*) FROM jcrs WHERE d @> '"a"'::jsonb) AS from_column;
