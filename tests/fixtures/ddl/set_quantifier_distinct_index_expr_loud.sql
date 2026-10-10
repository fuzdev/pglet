-- The third persisted-expression site — an index key part, which reaches `build_call`
-- through the same general expression grammar and so inherits the verdict. 42809 on
-- both.
-- @setup
CREATE TABLE sq_dix (c TEXT);
-- @test
-- @expect-error: DISTINCT specified, but upper is not an aggregate function
CREATE INDEX sq_dix_ix ON sq_dix ((upper(DISTINCT c)));
