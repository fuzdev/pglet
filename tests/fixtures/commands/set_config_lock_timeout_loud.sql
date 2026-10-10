-- pglet-stricter, and loud: `set_config` cannot move `lock_timeout`. The function lowers to an
-- expression the session never sees, so it answered the new value and `SHOW lock_timeout` went on
-- answering the old one — 18.6 moves the setting. The `SET` statement is the spelling pglet
-- acts on. Documentary: 18.6 accepts, so any pglet error passes; the SQLSTATE (`0A000`) is
-- pinned in the planner's unit tests.
-- @test
-- @expect-error: set_config cannot set lock_timeout; use SET
SELECT set_config('lock_timeout', '5s', false);
