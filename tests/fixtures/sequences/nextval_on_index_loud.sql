-- ...and an index name likewise, with the index wording.
-- @no-profile-lock: the error is raised resolving the sequence operand, before planning; no index is consulted on either profile.
-- @setup
CREATE TABLE nvi_host (a INT);
CREATE INDEX nvi_idx ON nvi_host (a);
-- @test
SELECT nextval('nvi_idx');
-- @expect-error: cannot open relation, not a sequence (42809)
