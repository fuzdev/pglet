-- @test
-- The visible form of the equality defect: three literals inside one microsecond
-- are one value, so `DISTINCT` folds them. pglet answered 3 — three rows printing
-- the identical string, which is the shape that reads as a display bug and is a
-- comparison one.
SELECT count(*) AS distinct_instants
FROM (
  SELECT DISTINCT t FROM (VALUES
    ('2026-01-01 00:00:00.1234561'::timestamp),
    ('2026-01-01 00:00:00.1234562'::timestamp),
    ('2026-01-01 00:00:00.1234563'::timestamp)
  ) v(t)
) d;
