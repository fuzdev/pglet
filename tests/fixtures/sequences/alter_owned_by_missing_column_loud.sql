-- `ALTER SEQUENCE ... OWNED BY <table>.<column>` resolves the column: 42703 when the
-- table lacks it.
-- @setup
CREATE TABLE aown_host (a INT);
CREATE SEQUENCE aown_seq;
-- @test
ALTER SEQUENCE aown_seq OWNED BY aown_host.nosuch;
-- @expect-error: column does not exist (42703)
