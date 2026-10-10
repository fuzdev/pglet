-- Both engines accept a `DEFAULT nextval('<the name this statement's own identity
-- column takes>')` — the before-list has already created that sequence by the time the
-- default is cooked, so the two columns share it. The identity column's `START WITH`
-- must survive: pglet registered a second, bare record for the name (its "is this
-- sequence already there?" test reads the catalog, which cannot see the in-flight one)
-- and, written first, that bare record won and the first id came out 1 instead of 100.
-- @setup
CREATE TABLE dnsis (b INT DEFAULT nextval('dnsis_a_seq'), a INT GENERATED ALWAYS AS IDENTITY (START WITH 100));
INSERT INTO dnsis (b) VALUES (1);
-- @test
SELECT a, b FROM dnsis;
