-- The same statement written the other way round answers the same.
-- @setup
CREATE SEQUENCE rst_combo2 START WITH 5;
ALTER SEQUENCE rst_combo2 RESTART START WITH 42;
-- @test
SELECT nextval('rst_combo2');
