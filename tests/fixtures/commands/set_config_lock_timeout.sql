-- `set_config` moves `lock_timeout` as `SET` / `SET LOCAL` do, and answers the value as
-- `SHOW` renders it: a later `current_setting` in the statement reads each assignment.
-- The fixture's block rolls the session value back on both engines.
-- @test
SELECT
  set_config('lock_timeout', '3600000', false) AS session_value,
  current_setting('lock_timeout') AS read_after,
  set_config('lock_timeout', '5s', true) AS local_value,
  current_setting('lock_timeout') AS read_local,
  set_config('lock_timeout', '0', false) AS zero;
