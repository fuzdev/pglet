-- @test
-- `count(DISTINCT …)` counts one value per set of equal ones, ignoring NULL: interval
-- day/hour and month/day spellings, `jsonb` number scales at any depth, `numeric[]`
-- element scales with NULL elements in place, NULL against the empty string, and a
-- float's signed zero and `NaN`s.
SELECT
  (SELECT count(DISTINCT x) FROM (VALUES (interval '1 day'), (interval '24 hours'), (interval '1 mon'), (interval '30 days'), (NULL)) v(x)) AS intervals,
  (SELECT count(DISTINCT x) FROM (VALUES ('{"a":1.0}'::jsonb), ('{"a":1}'), ('{"a":1.00, "b":[1.0]}'), ('{"b":[1], "a":1}')) v(x)) AS jsonb,
  (SELECT count(DISTINCT x) FROM (VALUES (ARRAY[1.0]), (ARRAY[1.00]), (ARRAY[1.0, NULL]), (ARRAY[NULL, 1.0]), (ARRAY[1.00, NULL])) v(x)) AS arrays,
  (SELECT count(DISTINCT x) FROM (VALUES (NULL::text), (''), (NULL), (''), ('a')) v(x)) AS texts,
  (SELECT count(DISTINCT x) FROM (VALUES ('-0'::float8), (0), ('NaN'), ('-NaN')) v(x)) AS floats;
