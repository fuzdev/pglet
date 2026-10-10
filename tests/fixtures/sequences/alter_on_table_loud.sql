-- A sequence-only statement whose target is a *table* is 42809, not 42P01 --
-- `validate_relation_kind` finds the relation and rejects its kind. pglet keeps
-- tables and sequences in separate maps, so answering 42P01 here is the easy mistake.
-- @setup
CREATE TABLE alter_not_a_seq (a INT);
-- @test
ALTER SEQUENCE alter_not_a_seq RESTART;
-- @expect-error: cannot open relation, not a sequence (42809)
