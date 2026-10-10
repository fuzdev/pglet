-- `interval × number` and `interval ÷ number`: the number casts to float8 and
-- fractions cascade down (month → days via 30, day → seconds via 86400).
-- @test
SELECT INTERVAL '1 mon' * 2.5   AS mul_frac,
       INTERVAL '2 days' * 1.5  AS mul_days,
       INTERVAL '1 day' * -1.5  AS mul_neg,
       INTERVAL '1 mon' / 2     AS div,
       INTERVAL '10 hours' / 4  AS div_hours;
