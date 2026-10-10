-- The other side of the split: `amcanunique` is Postgres's own check and it sits in
-- `DefineIndex` (`indexcmds.c:870`), ahead of `index_create`'s name conflict — so a
-- unique GIN over a taken name is the *method* error on both engines, not `42P07`.
-- Moving the whole method check below the name would break this pair as surely as
-- leaving the container half above it broke the other.
-- @setup
CREATE TABLE ciugbtn_t (a INT, j JSONB);
CREATE INDEX ciugbtn_ix ON ciugbtn_t (a);
-- @test
CREATE UNIQUE INDEX ciugbtn_ix ON ciugbtn_t USING gin (j);
-- @expect-error: a unique GIN is refused ahead of the taken name (0A000)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
