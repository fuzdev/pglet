-- The half of the same rule that matters most, because Postgres **succeeds** here: the
-- canonical idempotent-migration idiom skips on the taken name, and pglet's
-- container-key restriction must not turn that success into an error. Both key types
-- are pinned — the reorder reddened `jsonb` and `text[]` alike — and the count proves
-- the skip skipped rather than building a second index.
-- @setup
CREATE TABLE cinesock_t (a INT, j JSONB, arr TEXT[]);
CREATE INDEX cinesock_ix ON cinesock_t (a);
-- @test
CREATE INDEX IF NOT EXISTS cinesock_ix ON cinesock_t (j);
CREATE INDEX IF NOT EXISTS cinesock_ix ON cinesock_t (arr);
SELECT count(*)::INT AS n FROM pg_indexes WHERE tablename = 'cinesock_t';
