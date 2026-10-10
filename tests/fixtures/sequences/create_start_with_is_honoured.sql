-- `START WITH n` seeds the sequence so the first nextval returns n.
-- @setup
CREATE SEQUENCE start_seq START WITH 7;
-- @test
SELECT nextval('start_seq');
