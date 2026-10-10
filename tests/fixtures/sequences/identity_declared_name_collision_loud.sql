-- A declared name is refused, never disambiguated: the relation namespace already
-- holds it, so 42P07 — the asymmetry the auto-name's `t_a_seq1` is the other side of.
-- @setup
CREATE SEQUENCE idcol_taken;
-- @test
CREATE TABLE idcol (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME idcol_taken));
-- @expect-error: relation already exists (42P07)
