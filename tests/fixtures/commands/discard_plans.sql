-- `DISCARD PLANS` succeeds and is a no-op in both engines: Postgres's
-- `ResetPlanCache` throws away cached plans and keeps the prepared statements that
-- hold them, and pglet re-validates a cached plan against the catalog generation on
-- every use. It runs inside the harness's isolating block, where `DISCARD ALL` is
-- `25001`.
-- @test
DISCARD PLANS;
