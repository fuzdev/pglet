-- `week` is a recognized unit but is not supported for an interval (a month has no
-- fixed week count), so date_trunc('week', interval) is 0A000 (feature_not_supported)
-- in both pglet and Postgres.
-- @test
-- @expect-error: unit "week" not supported for type interval
SELECT date_trunc('week', INTERVAL '5 days 3 hours');
