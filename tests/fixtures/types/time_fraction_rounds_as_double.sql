-- A `time` fraction past six digits rounds as 18.6's `rint` over a C `double`, not as a
-- decimal: `.0001255` is not a tie once it is a `double` and rounds down, `.0001265`
-- rounds up.
-- @test
SELECT
  '12:00:00.0001255'::time AS rounds_down,
  '12:00:00.0001265'::time AS rounds_up,
  '12:00:00.0000025'::time AS tie_to_even
;
