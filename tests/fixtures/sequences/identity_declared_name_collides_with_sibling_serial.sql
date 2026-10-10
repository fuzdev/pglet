-- Two implicit sequences in one CREATE TABLE can collide, and it is 42P07 rather than
-- a disambiguation: `ChooseRelationName` reads the committed catalog only, so the
-- SERIAL below picks the name the identity column above already spoke for and the
-- second CREATE SEQUENCE in the before-list fails.
-- @test
CREATE TABLE idsib (b INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idsib_a_seq), a SERIAL);
-- @expect-error: relation already exists (42P07)
