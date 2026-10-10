-- The spelling `pg_dump --clean --if-exists` writes for every constraint. Over a table
-- that is not there it is the skip — which is what keeps a `--clean` script loading into
-- an empty database — and the refusal of the drop itself is reached only over a table
-- that exists.
-- @test
ALTER TABLE IF EXISTS ONLY nosuch_atie17 DROP CONSTRAINT IF EXISTS atie17_pkey;
