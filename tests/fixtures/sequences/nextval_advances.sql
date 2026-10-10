-- nextval advances the sequence and returns the new value. Setup takes the
-- first value (1); the test takes the next (2), demonstrating advancement.
-- @setup
CREATE SEQUENCE seq_nextval;
SELECT nextval('seq_nextval');
-- @test
SELECT nextval('seq_nextval');
