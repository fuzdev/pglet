-- @test
-- `timestamptz` reads the same parser through its offset-applying arm, so the
-- microsecond rounding has to be in the shared body and not on the naive path
-- alone. The harness pins both sessions to `TimeZone=UTC`.
SELECT '2026-01-01 00:00:00.1234566+00'::timestamptz AS rounded;
