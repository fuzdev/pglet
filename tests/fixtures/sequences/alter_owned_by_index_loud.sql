-- An OWNED BY naming an index is 42809, and with `process_owned_by`'s **own** wording
-- (`sequence cannot be owned by relation`) rather than the target path's `cannot open
-- relation` — the statement's two relation operands are checked separately.
-- @no-profile-lock: the error is raised resolving the owner operand, before planning; no index is consulted on either profile.
-- @setup
CREATE TABLE aobi_host (a INT);
CREATE INDEX aobi_idx ON aobi_host (a);
CREATE SEQUENCE aobi_seq;
-- @test
ALTER SEQUENCE aobi_seq OWNED BY aobi_idx.a;
-- @expect-error: sequence cannot be owned by an index (42809)
