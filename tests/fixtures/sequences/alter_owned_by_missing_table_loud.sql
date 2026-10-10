-- ...and 42P01 when the owner table does not exist.
-- @setup
CREATE SEQUENCE aown_seq2;
-- @test
ALTER SEQUENCE aown_seq2 OWNED BY nosuch_owner_table.a;
-- @expect-error: relation does not exist (42P01)
