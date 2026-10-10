-- `DefineIndex` opens the target and switches on its relkind before anything else, so
-- an index as the *target* of `CREATE INDEX` is `42809`.
-- @setup
CREATE TABLE cioi_t (a INT);
CREATE INDEX cioi_ix ON cioi_t (a);
-- @test
CREATE INDEX cioi_ix2 ON cioi_ix (a);
-- @expect-error: an index is not an indexable relation (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
