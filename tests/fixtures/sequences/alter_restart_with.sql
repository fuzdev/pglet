-- `RESTART WITH n` makes the next nextval return exactly n.
-- @setup
CREATE SEQUENCE rst_with START WITH 5;
SELECT nextval('rst_with');
ALTER SEQUENCE rst_with RESTART WITH 100;
-- @test
SELECT nextval('rst_with');
