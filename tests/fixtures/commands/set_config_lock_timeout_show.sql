-- A session `set_config` of `lock_timeout` is kept past its statement, as 18.6 keeps it:
-- the `SHOW` after it reads the assigned value. The fixture's block rolls it back.
-- @test
SELECT set_config('lock_timeout', '7s', false);
SHOW lock_timeout;
