-- The index-**predicate** spelling of the unrenderable interval literal. The
-- predicate is over an `INTERVAL` column rather than `now()`, deliberately: 18.6
-- requires IMMUTABLE in an index position (`42P17`), so a `now()`-bearing predicate
-- would be refused there for a different reason and could not tell this gate from
-- that one. `interval > interval` is immutable and 18.6 accepts this statement.
-- @no-profile-lock: DDL `@test` — index selection never runs, and the render probe is not feature-gated, so both profiles share one path
-- @expect-error: pglet-only — an interval literal in a stored expression cannot render
-- @setup
CREATE TABLE iv_pred (iv INTERVAL, n INT);
-- @test
CREATE INDEX iv_pred_n ON iv_pred (n) WHERE iv > interval '1 day';
