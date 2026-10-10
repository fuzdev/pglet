-- The skip fires only for a name nothing holds — the accept side of the wrong-kind
-- rule below it. The counter read afterwards proves the statement ran and stopped
-- there rather than disturbing the surviving sequence.
-- @setup
CREATE SEQUENCE dsie_seq START WITH 3;
DROP SEQUENCE IF EXISTS dsie_nope;
-- @test
SELECT nextval('dsie_seq');
