-- A foreign key's parent is opened and checked for kind: `referenced relation "s" is
-- not a table`. On the CREATE TABLE route the FK block resolves the parent, so this is
-- the same 42809 the write shapes give.
-- @setup
CREATE SEQUENCE refs_seq;
-- @test
CREATE TABLE refs_child (b INT REFERENCES refs_seq(a));
-- @expect-error: a sequence cannot be referenced (42809)
-- @no-error-fields: 18.6 answers this route with `referenced relation "x" is not a table` and no DETAIL; pglet reaches it through the `cannot open relation` wording, which carries one
