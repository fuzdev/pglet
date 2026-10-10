-- ...and in the other written order, where the SERIAL's auto-name is claimed first.
-- @test
CREATE TABLE idsib2 (a SERIAL, b INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idsib2_a_seq));
-- @expect-error: relation already exists (42P07)
