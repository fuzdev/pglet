-- ...and a sequence owner is the third kind, 42809 with "not supported for sequences".
-- @setup
CREATE SEQUENCE aobs_seq;
CREATE SEQUENCE aobs_other;
-- @test
ALTER SEQUENCE aobs_seq OWNED BY aobs_other.a;
-- @expect-error: sequence cannot be owned by a sequence (42809)
