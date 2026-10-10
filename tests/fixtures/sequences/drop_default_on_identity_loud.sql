-- The `DROP DEFAULT` spelling reaches the same gate (Postgres adds a hint pointing at
-- `DROP IDENTITY`; the SQLSTATE is the same 42601). It needs its own fixture because
-- it arrives at the executor as a `SET DEFAULT` with no value, so a gate written
-- against the *default expression* rather than the column skips it — and clearing the
-- link is the worse half: the column keeps its identity and its NOT NULL with nothing
-- left to fill it.
-- @setup
CREATE TABLE ddid_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER TABLE ddid_t ALTER COLUMN id DROP DEFAULT;
-- @expect-error: column is an identity column (42601)
