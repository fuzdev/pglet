-- The owner's *kind* check runs before its column lookup: an index owner is 42809
-- even when the named column would also be missing.
-- @no-profile-lock: the error is raised resolving the owner operand, before planning; no index is consulted on either profile.
-- @setup
CREATE TABLE aobk_host (a INT);
CREATE INDEX aobk_idx ON aobk_host (a);
CREATE SEQUENCE aobk_seq;
-- @test
ALTER SEQUENCE aobk_seq OWNED BY aobk_idx.nosuch;
-- @expect-error: wrong owner kind outranks the missing column (42809)
