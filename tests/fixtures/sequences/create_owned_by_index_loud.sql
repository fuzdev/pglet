-- The owner kind check is on the CREATE path too (`process_owned_by` is shared).
-- @no-profile-lock: the error is raised resolving the owner operand, before planning; no index is consulted on either profile.
-- @setup
CREATE TABLE cobi_host (a INT);
CREATE INDEX cobi_idx ON cobi_host (a);
-- @test
CREATE SEQUENCE cobi_seq OWNED BY cobi_idx.a;
-- @expect-error: sequence cannot be owned by an index (42809)
