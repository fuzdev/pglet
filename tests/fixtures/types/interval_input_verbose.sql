-- INTERVAL input, verbose grammar: multi-unit spans and Postgres's fractional
-- normalization (years round to whole months; weeks/months spill into days then
-- microseconds). Interval now projects, so its `postgres`-style text is
-- differential-tested in both wire formats.
-- @test
SELECT INTERVAL '1 year 2 mons 3 days 04:05:06'                               AS multi,
       INTERVAL '2 years 15 months 100 weeks 99 hours 123456789 milliseconds' AS canonical,
       INTERVAL '1.5 years'                                                    AS frac_years,
       INTERVAL '1.75 months'                                                  AS frac_months,
       INTERVAL '90 minutes'                                                   AS ninety_min;
