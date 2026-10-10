-- The declared-type threading that keeps a stringified `timestamptz` carrying its
-- `+00` offset has to reach the **aggregate** path too: `max('v=' || a)` evaluates
-- the concat through the aggregate argument evaluator, and `'v=' || max(a)`
-- through the aggregate-context binary-op evaluator. Both once dropped the offset
-- (the same silent instant shift `concat_timestamptz` guards on the scalar path).
-- @setup
CREATE TABLE ctza (a TIMESTAMPTZ, b TIMESTAMP);
INSERT INTO ctza VALUES
  ('2026-07-18 12:30:45+00', '2026-07-18 12:30:45'),
  ('2026-07-19 01:02:03+00', '2026-07-19 01:02:03');
-- @test
SELECT
  max('v=' || a)  AS concat_in_agg,
  max(a::text)    AS cast_in_agg,
  'w=' || max(a)  AS agg_in_concat,
  max(a)::text    AS agg_then_cast,
  max('p=' || b)  AS plain_concat_in_agg
FROM ctza;
