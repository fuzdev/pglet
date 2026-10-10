-- `greatest`'s eval-time fold was the third raw caller. Postgres resolves
-- `greatest`/`least` with `select_common_type`, so a `time`/`timestamp` mix is `42846`
-- — the same code the set-op and `VALUES` paths give, and the reason all three now
-- share one fold rather than each keeping its own.
-- @setup
CREATE TABLE gtt (tm TIME, ts TIMESTAMP);
-- The `time` column is NULL on purpose. `greatest`/`least` drop NULL arguments
-- before folding at eval time, so a row like this is exactly what a runtime-only
-- check cannot see — it would fold the single surviving argument and answer. Only a
-- static check reports the error Postgres reports regardless of the data.
INSERT INTO gtt VALUES (NULL, '2026-01-01 10:00:00');
-- @test
-- @expect-error: no common type for time and timestamp (42846)
SELECT greatest(tm, ts) FROM gtt;
