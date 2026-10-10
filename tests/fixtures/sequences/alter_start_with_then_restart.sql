-- ...and after the RESTART it lands on the new start.
-- @setup
CREATE SEQUENCE rst_start2 START WITH 5;
SELECT nextval('rst_start2');
ALTER SEQUENCE rst_start2 START WITH 30;
ALTER SEQUENCE rst_start2 RESTART;
-- @test
SELECT nextval('rst_start2');
