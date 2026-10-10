-- The vacuous and JSON-`null` cases. An empty rhs container is contained by any
-- container of the same kind (nothing to find), and a JSON `null` is an ordinary
-- **value** that matches by equality — not SQL NULL, which makes the whole operator
-- return SQL NULL.
--
-- `'[1,2] @> []'` was false in pglet (the old code compared the two arrays
-- structurally), which is the vacuous-truth case a containment implementation is
-- most likely to miss.
-- @setup
CREATE TABLE jcen (id INT, d JSONB);
INSERT INTO jcen VALUES (1, '[null,1]'), (2, '{"a":null}'), (3, NULL);
-- @test
SELECT
  ('[1,2]'::jsonb @> '[]'::jsonb) AS empty_array_rhs,
  ('{"a":1}'::jsonb @> '{}'::jsonb) AS empty_object_rhs,
  ('[]'::jsonb @> '[]'::jsonb) AS empty_both,
  ('[null,1]'::jsonb @> '[null]'::jsonb) AS json_null_element,
  ('{"a":null}'::jsonb @> '{"a":null}'::jsonb) AS json_null_value,
  ('{"a":null}'::jsonb @> '{"a":1}'::jsonb) AS json_null_is_not_one,
  (NULL::jsonb @> '[]'::jsonb) IS NULL AS sql_null_propagates,
  (SELECT count(*) FROM jcen WHERE d @> '[null]'::jsonb) AS from_column;
