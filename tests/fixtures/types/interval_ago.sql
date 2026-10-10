-- The trailing `ago` keyword negates every field of the interval.
-- @test
SELECT INTERVAL '1 day 2 hours ago'          AS day_time_ago,
       INTERVAL '1 year 2 mons 3 days ago'   AS ymd_ago,
       INTERVAL '3 hours ago'                AS hours_ago;
