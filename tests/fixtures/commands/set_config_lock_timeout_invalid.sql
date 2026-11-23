-- A value `lock_timeout` refuses is 18.6's `22023`, raised as the call is evaluated.
-- @test
-- @expect-error: invalid value for parameter "lock_timeout"
SELECT set_config('lock_timeout', 'soon', false);
