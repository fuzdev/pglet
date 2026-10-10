-- Written together, START WITH applies first whatever the order -- the options are
-- collected and then applied, not executed left to right.
-- @setup
CREATE SEQUENCE rst_combo START WITH 5;
ALTER SEQUENCE rst_combo START WITH 42 RESTART;
-- @test
SELECT nextval('rst_combo');
