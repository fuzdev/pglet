-- The SAME `time` vs `interval` comparisons as `compare_time_interval.sql`, but over
-- an **indexed** column, so the accelerated build lifts each predicate into an index
-- probe instead of evaluating it. `interval → time` is only an *assignment* cast and
-- is lossy (`INTERVAL '1 day'` has no time-of-day), so no `time` key value stands in
-- for the interval: the probe must decline and fall back to a filtered scan rather
-- than key on a coerced-or-raw value. Before that, the accelerated build answered
-- `=`/`>` with zero rows and `<=` with an extra one, while the stripped build was
-- right — the exact divergence the byte-identical-profiles contract forbids.
-- @profile-lock
-- @setup
CREATE TABLE t (tm TIME, tag TEXT);
INSERT INTO t VALUES ('01:00:00', 'a'), ('02:00:00', 'b');
CREATE INDEX t_tm ON t (tm);
-- @test
SELECT 'eq' AS op, tag FROM t WHERE tm = INTERVAL '1 hour'
UNION ALL
SELECT 'gt', tag FROM t WHERE tm > INTERVAL '1 hour'
UNION ALL
SELECT 'le', tag FROM t WHERE tm <= INTERVAL '1 hour'
ORDER BY 1, 2;
