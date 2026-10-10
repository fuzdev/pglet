-- `DEFAULT nextval('<an enum>')`: `'x'::regclass` finds no relation, so 18.6 answers
-- `42P01` at the cook. A name nothing holds is auto-created as a sequence here, which
-- over an enum would put a sequence and a type under one name.
-- @setup
CREATE TYPE tnt16s AS ENUM ('x');
-- @expect-error: nextval of an enum's name
-- @expect-message
-- @test
CREATE TABLE tnt16 (a INT DEFAULT nextval('tnt16s'));
