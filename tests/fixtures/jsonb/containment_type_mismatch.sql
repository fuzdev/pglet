-- The negative half: containment is **not** permissive. Widening `@>` from
-- object-only to Postgres's full rules must not start matching pairs Postgres
-- refuses, which is the failure mode a subset-flavoured implementation drifts into.
--
-- Each of these is false in Postgres for a distinct reason: a value's type must
-- match exactly (`1` is not `"1"`), object and array kinds never cross, a scalar
-- contains no container, and a key the left operand lacks fails outright.
-- @setup
CREATE TABLE jctm (id INT, d JSONB);
INSERT INTO jctm VALUES (1, '{"a":1}');
-- @test
SELECT
  ('{"a":1}'::jsonb @> '{"a":"1"}'::jsonb) AS number_is_not_string,
  ('{"a":1}'::jsonb @> '[1]'::jsonb) AS object_vs_array,
  ('[1]'::jsonb @> '{"a":1}'::jsonb) AS array_vs_object,
  ('{"a":1}'::jsonb @> '{"a":1,"b":2}'::jsonb) AS missing_key,
  ('{"a":true}'::jsonb @> '{"a":1}'::jsonb) AS bool_is_not_number,
  ('1'::jsonb @> '[1]'::jsonb) AS scalar_vs_array,
  ('{"a":1.0}'::jsonb @> '{"a":1}'::jsonb) AS numeric_scale_folds,
  (SELECT count(*) FROM jctm WHERE d @> '{"a":"1"}'::jsonb) AS from_column;
