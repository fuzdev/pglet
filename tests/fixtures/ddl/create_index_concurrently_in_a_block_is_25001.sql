-- `CREATE INDEX CONCURRENTLY` is the one thing the keyword decides here: 18.6
-- refuses it inside a transaction block with `25001`, and pglet — single-writer, so
-- with no concurrent build to perform — reproduces exactly that refusal and nothing
-- else. The harness's own isolating `BEGIN … ROLLBACK` **is** the block, so this
-- fixture needs no transaction of its own.
-- @setup
CREATE TABLE ti (id INT, v TEXT);
-- @test
-- @no-profile-lock: the `25001` refusal runs before the plan is executed and before
-- any index is built, so index selection never runs on either profile — the
-- completeness check is reading the `CREATE INDEX` in this comment block, not one in
-- the setup.
-- @expect-error: CREATE INDEX CONCURRENTLY cannot run inside a transaction block
CREATE INDEX CONCURRENTLY ti_id_idx ON ti (id);
