-- The zero-row case of the static aggregate gate: `WHERE false` observes no value
-- at all, and Postgres still rejects `min(boolean)` at parse time.
-- @setup
CREATE TABLE agg_norows (b BOOL);
-- @test
-- @expect-error: no min(boolean)
SELECT MIN(b) FROM agg_norows WHERE false;
