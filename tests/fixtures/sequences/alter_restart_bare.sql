-- A bare `RESTART` returns the counter to the sequence's START WITH, which a lone
-- counter cannot compute -- this is the behaviour that forced the sequence to become
-- a catalog record. The whole statement used to be a no-op.
-- @setup
CREATE SEQUENCE rst_bare START WITH 5;
SELECT nextval('rst_bare');
SELECT nextval('rst_bare');
ALTER SEQUENCE rst_bare RESTART;
-- @test
SELECT nextval('rst_bare');
