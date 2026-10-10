-- `CURRENT_TIME` / `LOCALTIME` — the SQL value functions (parenless). Their value
-- is a moving clock, so compare only deterministic properties. `CURRENT_TIME` is
-- `timetz` in Postgres but a plain `time` in pglet (UTC-only), so use a type-agnostic
-- `IS NOT NULL` for it; `LOCALTIME` is `time` on both engines, so its range bounds
-- compare identically. All resolve to a deterministic `true`.
-- @test
SELECT CURRENT_TIME IS NOT NULL      AS current_time_set,
       LOCALTIME IS NOT NULL         AS localtime_set,
       LOCALTIME >= '00:00:00'::time  AS after_midnight,
       LOCALTIME <= '24:00:00'::time  AS before_end;
