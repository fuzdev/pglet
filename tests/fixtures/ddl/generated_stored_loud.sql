-- A `GENERATED ALWAYS AS (<expr>) STORED` computed column is not modeled by pglet
-- — it fails loud (0A000, feature_not_supported) rather than silently mis-parsing.
-- Postgres accepts it, so this is a deliberately pglet-stricter case (any pglet
-- error passes).
-- @expect-error: pglet does not support GENERATED … STORED computed columns
-- @test
CREATE TABLE t (a int, b int GENERATED ALWAYS AS (a * 2) STORED);
