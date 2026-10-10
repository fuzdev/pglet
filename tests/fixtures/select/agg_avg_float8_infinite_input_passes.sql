-- @test
-- The accept side of `avg`'s overflow check: once an input is infinite the running
-- sum is too and no overflow is reported, whatever follows; a `NaN` input passes; and
-- a sum of squares that stays in range passes.
SELECT
  (SELECT avg(x)::text FROM (VALUES ('Infinity'::float8), (1e308), (1e308)) v(x)) AS inf_first,
  (SELECT avg(x)::text FROM (VALUES ('NaN'::float8), (1e308), (1e308)) v(x)) AS nan_first,
  (SELECT sum(x)::text FROM (VALUES ('Infinity'::float8), (1e308), (1e308)) v(x)) AS sum_inf_first,
  (SELECT avg(x) FROM (VALUES (1e150::float8), (-1e150)) v(x)) AS squares_in_range;
