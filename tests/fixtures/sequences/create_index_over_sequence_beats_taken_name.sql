-- The same ordering without `IF NOT EXISTS`: the sequence target (42809) outranks the
-- index name conflict (42P07) `index_create` would raise last.
-- @setup
CREATE SEQUENCE cist_seq;
CREATE TABLE cist_t (a INT);
CREATE INDEX cist_ix ON cist_t (a);
-- @test
CREATE INDEX cist_ix ON cist_seq (a);
-- @expect-error: the target's kind outranks the taken index name (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
