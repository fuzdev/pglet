-- `DROP CONSTRAINT` over a sequence is the wrong relation kind, `42809`, and so it is
-- under `IF EXISTS` too: the name resolves, so there is nothing to skip.
-- @expect-error: a sequence is not a table
-- @setup
CREATE SEQUENCE dcos_seq;

-- @test
ALTER TABLE IF EXISTS dcos_seq DROP CONSTRAINT IF EXISTS x;
