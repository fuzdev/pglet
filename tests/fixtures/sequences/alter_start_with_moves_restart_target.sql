-- `START WITH n` on its own moves the *recorded* start without touching the counter,
-- so a later bare RESTART goes to the new value. Tolerating this clause as a no-op
-- while honouring RESTART would be silently wrong.
-- @setup
CREATE SEQUENCE rst_start START WITH 5;
SELECT nextval('rst_start');
ALTER SEQUENCE rst_start START WITH 30;
-- @test
SELECT nextval('rst_start') AS not_yet_restarted;
