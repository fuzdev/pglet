-- `isfinite` answers false for either infinity, and `to_char` of an infinity is NULL
-- whatever the format.
-- @test
SELECT isfinite(date 'infinity') AS a, isfinite(timestamp '-infinity') AS b,
       isfinite(timestamptz '2000-01-01 00:00:00+00') AS c, isfinite(date '9999-12-31') AS d,
       isfinite(NULL::date) AS e, to_char(timestamp 'infinity', 'YYYY') AS f,
       to_char(timestamptz '-infinity', 'YYYY-MM-DD"T"HH24:MI:SS"Z"') AS g,
       to_char(timestamp 'infinity', 'bogus') AS h;
