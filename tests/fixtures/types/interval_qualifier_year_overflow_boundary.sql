-- The largest year count a `YEAR` qualifier accepts, and the value on the accepting
-- side of the boundary. The refusing side is
-- `interval_qualifier_year_overflow_is_loud.sql`; the pair is the point, because a
-- clamp is invisible from either side alone — every count above the boundary used to
-- answer this same `178956970 years`, silently, since the year multiply saturated
-- instead of failing.
-- @test
SELECT
  INTERVAL '178956970' YEAR AS max_years,
  INTERVAL '-178956970' YEAR AS min_years,
  INTERVAL '2147483647' MONTH AS max_months,
  INTERVAL '-2147483648' MONTH AS min_months,
  INTERVAL '2147483647' DAY AS max_days,
  INTERVAL '-2147483648' DAY AS min_days,
  INTERVAL '2562047788' HOUR AS max_hours,
  INTERVAL '153722867280' MINUTE AS max_minutes,
  INTERVAL '9223372036854' SECOND AS max_seconds;
