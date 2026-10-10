-- @test
-- `GROUP BY` groups by the equality `SELECT DISTINCT` dedups by: a `jsonb` number's
-- scale, a `numeric[]` element's scale, an `interval[]` element's day/hour and
-- month/day spellings and a `float8[]` element's signed zero each fold into one group
-- of two, in both input orders, and the group shows the spelling met first. A second
-- group would make its scalar subquery return two rows and fail.
SELECT
  (SELECT x::text || ' x' || count(*) FROM (VALUES ('{"a":1.0}'::jsonb), ('{"a":1}')) v(x) GROUP BY x) AS jsonb_a,
  (SELECT x::text || ' x' || count(*) FROM (VALUES ('{"a":1}'::jsonb), ('{"a":1.0}')) v(x) GROUP BY x) AS jsonb_b,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[1.0]), (ARRAY[1.00])) v(x) GROUP BY x) AS array_a,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[1.00]), (ARRAY[1.0])) v(x) GROUP BY x) AS array_b,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[interval '1 day']), (ARRAY[interval '24 hours'])) v(x) GROUP BY x) AS day_a,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[interval '24 hours']), (ARRAY[interval '1 day'])) v(x) GROUP BY x) AS day_b,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[interval '1 mon']), (ARRAY[interval '30 days'])) v(x) GROUP BY x) AS mon_a,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[interval '30 days']), (ARRAY[interval '1 mon'])) v(x) GROUP BY x) AS mon_b,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY[0::float8]), (ARRAY['-0'::float8])) v(x) GROUP BY x) AS zero_a,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY['-0'::float8]), (ARRAY[0::float8])) v(x) GROUP BY x) AS zero_b,
  (SELECT x::text || ' x' || count(*) FROM (VALUES (ARRAY['NaN'::float8]), (ARRAY[-'NaN'::float8])) v(x) GROUP BY x) AS nan;
