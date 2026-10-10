-- currval reports the last value nextval returned for this sequence in this
-- session (the setup nextval sets it).
-- @setup
CREATE SEQUENCE seq_currval;
SELECT nextval('seq_currval');
-- @test
SELECT currval('seq_currval');
