-- @test
-- `SELECT DISTINCT` keeps the first row of each set of equal values, in both input
-- orders: a `numeric` display scale, an interval's day/hour and month/day spellings,
-- a float's signed zero, a `jsonb` number's scale and a `numeric[]` element's scale
-- each fold into one row whose spelling is the one met first.
SELECT
  (SELECT DISTINCT x FROM (VALUES (1.0::numeric), (1.00)) v(x)) AS numeric_a,
  (SELECT DISTINCT x FROM (VALUES (1.00::numeric), (1.0)) v(x)) AS numeric_b,
  (SELECT DISTINCT x FROM (VALUES (interval '1 day'), (interval '24 hours')) v(x)) AS day_a,
  (SELECT DISTINCT x FROM (VALUES (interval '24 hours'), (interval '1 day')) v(x)) AS day_b,
  (SELECT DISTINCT x FROM (VALUES (interval '1 mon'), (interval '30 days')) v(x)) AS mon_a,
  (SELECT DISTINCT x FROM (VALUES (interval '30 days'), (interval '1 mon')) v(x)) AS mon_b,
  (SELECT DISTINCT x FROM (VALUES ('-0'::float8), (0)) v(x)) AS zero_a,
  (SELECT DISTINCT x FROM (VALUES (0::float8), ('-0')) v(x)) AS zero_b,
  (SELECT DISTINCT x FROM (VALUES ('{"a":1.0}'::jsonb), ('{"a":1}')) v(x)) AS jsonb_a,
  (SELECT DISTINCT x FROM (VALUES ('{"a":1}'::jsonb), ('{"a":1.0}')) v(x)) AS jsonb_b,
  (SELECT DISTINCT x FROM (VALUES (ARRAY[1.0]), (ARRAY[1.00])) v(x)) AS array_a,
  (SELECT DISTINCT x FROM (VALUES (ARRAY[1.00]), (ARRAY[1.0])) v(x)) AS array_b;
