-- kysely 0.28's `PostgresAdapter` migration lock, as its `Migrator` issues it: the lock
-- with a local `lock_timeout` over a CTE, then the unlock — `true`, the session holding
-- the lock the first statement took, which the fixture's block ending does not release
-- (a session-level lock outlives its block); the unlock gives it back.
-- @test
with set_timeout as (
  select set_config('lock_timeout', '3600000', true) as config_val
)
select pg_advisory_lock(3853314791062309107)
from set_timeout;
select pg_advisory_unlock(3853314791062309107);
