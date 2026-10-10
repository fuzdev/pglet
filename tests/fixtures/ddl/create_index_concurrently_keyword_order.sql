-- The keyword's position is fixed, not merely tolerated: `CONCURRENTLY` sits
-- between `INDEX` and `IF NOT EXISTS`, and 18.6 answers the reversed spelling with
-- `42601 syntax error at or near "CONCURRENTLY"`.
-- @setup
CREATE TABLE ti (id INT);
--
-- **A lock, not a new assertion.** It is green against an engine that does not know
-- the keyword at all, where every `CONCURRENTLY` spelling is `42601` from the generic
-- "expected `ON`" path rather than from the keyword's position. What it holds is that
-- admitting the keyword did not admit the reversed order with it.
-- @test
-- @expect-error: syntax error at or near "CONCURRENTLY"
-- @no-error-fields: both engines send `P` and point at different tokens — 18.6 rejects at CONCURRENTLY, pglet after the index name
CREATE INDEX IF NOT EXISTS CONCURRENTLY ti_id_idx ON ti (id);
