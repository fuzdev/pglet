-- The index **key expression** spelling of the unrenderable interval literal, over
-- an `INTERVAL` column for the same immutability reason as
-- `index_predicate_interval_literal_unrenderable_loud.sql`: `interval + interval` is
-- immutable and 18.6 builds this index.
-- @no-profile-lock: DDL `@test` — index selection never runs, and the render probe is not feature-gated, so both profiles share one path
-- @expect-error: pglet-only — an interval literal in a stored expression cannot render
-- @setup
CREATE TABLE iv_key (iv INTERVAL, n INT);
-- @test
CREATE INDEX iv_key_shift ON iv_key ((iv + interval '1 day'));
