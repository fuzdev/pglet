-- `set_config` returns the setting **as it now stands**, not its argument: 18.6 answers `UTC`
-- for `'utc'`. pglet's value for this parameter is fixed, so an accepted spelling answers that
-- value. A value comparison, and the accept side of the gate on this route.
-- @test
SELECT set_config('TimeZone', 'utc', false) AS v;
