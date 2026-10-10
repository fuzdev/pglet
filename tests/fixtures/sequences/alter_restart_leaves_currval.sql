-- A RESTART does not disturb `currval`, which is per-session state living outside
-- the catalog: in-session it still reports the pre-restart value.
-- @setup
CREATE SEQUENCE rst_currval START WITH 5;
SELECT nextval('rst_currval');
SELECT nextval('rst_currval');
ALTER SEQUENCE rst_currval RESTART WITH 100;
-- @test
SELECT currval('rst_currval');
