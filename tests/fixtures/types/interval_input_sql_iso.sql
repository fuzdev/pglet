-- INTERVAL input, SQL-standard (`Y-M` year-month, `D H:M:S` day + time) and
-- ISO-8601 (`P…T…`, where `M` before T is months and after T is minutes) grammars.
-- A leading sign on the `Y-M` form applies to both parts (`-1-2` = -14 months).
-- @test
SELECT INTERVAL '1-2'            AS year_month,
       INTERVAL '-1-2'           AS neg_year_month,
       INTERVAL '+1-2'           AS pos_year_month,
       INTERVAL '3 4:05:06'      AS day_time,
       INTERVAL '-1 2:03:04'     AS neg_day_pos_time,
       INTERVAL 'P1Y2M3DT4H5M6S' AS iso_full,
       INTERVAL 'PT5M'           AS iso_minutes;
