-- `IF NOT EXISTS` on an existing sequence is a skip: the *existing* sequence's
-- START WITH is what the first nextval returns, not the re-created one's.
-- @setup
CREATE SEQUENCE ine_seq START WITH 5;
CREATE SEQUENCE IF NOT EXISTS ine_seq START WITH 50;
-- @test
SELECT nextval('ine_seq');
