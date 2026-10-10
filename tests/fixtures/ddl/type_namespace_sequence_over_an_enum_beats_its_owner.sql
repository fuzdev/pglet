-- The heap is created before `process_owned_by` resolves the owner, so the type
-- namespace's `42710` outranks a missing owner table's `42P01`.
-- @setup
CREATE TYPE tnt6 AS ENUM ('x');
-- @expect-error: the type collision outranks the missing owner
-- @expect-message
-- @test
CREATE SEQUENCE tnt6 OWNED BY nosuch_tnt6.a;
