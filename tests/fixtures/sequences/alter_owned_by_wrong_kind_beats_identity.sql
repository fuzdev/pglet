-- ...and so does the owner's kind check, the first of the three. The three fixtures
-- together pin the whole prefix: an identity sequence's `OWNED BY` reports the
-- *owner's* problem whenever the owner has one, and the 0A000 only when it does not.
-- @no-profile-lock: the error is raised resolving the owner operand, before planning; no index is consulted on either profile.
-- @setup
CREATE TABLE aobwk_host (h INT);
CREATE INDEX aobwk_idx ON aobwk_host (h);
CREATE TABLE aobwk_t (id INT GENERATED ALWAYS AS IDENTITY, b INT);
-- @test
ALTER SEQUENCE aobwk_t_id_seq OWNED BY aobwk_idx.h;
-- @expect-error: the wrong-kind owner outranks the identity refusal (42809)
