-- The handler twin of `do_block_index_concurrently_is_25001.sql`: a `DO` handler's
-- statements pass the same `25001` gate its body's do (`check_allowed_in_do_block`),
-- so `CREATE INDEX CONCURRENTLY` in a handler is refused as 18.6 refuses it from a
-- function, and builds no index. When the handler ran outside that gate the caught
-- body's error was swallowed and the index was built.
-- @setup
CREATE TABLE thc (c INT);
-- @test
-- @no-profile-lock: the `25001` refusal precedes execution and builds no index, so
-- index selection never runs on either profile — the completeness check is reading
-- the `CREATE INDEX` in this comment block, not one in the setup.
-- @expect-error: CREATE INDEX CONCURRENTLY cannot run inside a transaction block
DO $$ BEGIN INSERT INTO thc_nosuch VALUES (1);
EXCEPTION WHEN OTHERS THEN CREATE INDEX CONCURRENTLY thc_c_idx ON thc (c); END $$;
