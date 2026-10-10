-- The other shape a runtime-only check misses entirely: with **no rows**, the eval-time
-- fold never runs, so the query was accepted and returned an empty result. Postgres
-- rejects it at parse analysis, before it ever looks at data.
-- @setup
CREATE TABLE gem (i4 INT, t TEXT);
-- @test
-- @expect-error: rejected at plan time, with no rows to fold (42804)
SELECT GREATEST(i4, t) FROM gem;
