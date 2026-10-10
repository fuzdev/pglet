-- ...over the whole relation namespace, not just the sequences.
-- @setup
CREATE TABLE idcolt_taken (a INT);
-- @test
CREATE TABLE idcolt (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idcolt_taken));
-- @expect-error: relation already exists (42P07)
