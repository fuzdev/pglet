-- `set_config` returns the setting **as it now stands**, not its argument: 18.6 answers `UTC`
-- for `'utc'`. pglet's value for this parameter is fixed, so an accepted spelling answers that
-- value. A value comparison, with `is_local` true, which does not enter the decision.
-- @test
SELECT set_config('timezone', 'utc', true) AS v;
