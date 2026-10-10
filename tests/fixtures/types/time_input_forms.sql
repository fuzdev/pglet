-- The accepted TIME input spellings and their canonical rendering: colon forms,
-- the all-digit ISO basic forms (`HHMMSS`/`HHMM`), the 12-hour AM/PM form, the
-- trailing-zero fraction trim, and the no-fraction form — differential vs Postgres.
-- @test
SELECT '13:45:06.123456'::time AS micros,
       '13:45:06'::time        AS hms,
       '13:45'::time           AS hm,
       '134506'::time          AS basic6,
       '1345'::time            AS basic4,
       '12:00 AM'::time        AS midnight,
       '12:00 PM'::time        AS noon,
       '4:00 PM'::time         AS four_pm,
       '00:00:00.100000'::time AS trim_frac,
       '13:45:06.120000'::time AS trim_frac2,
       '134506.5'::time        AS basic6_frac;
