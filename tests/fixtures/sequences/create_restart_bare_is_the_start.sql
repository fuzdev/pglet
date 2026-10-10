-- A bare RESTART on CREATE is the seeding `isInit` already does, so START WITH wins.
-- @setup
CREATE SEQUENCE crst3 START WITH 7 RESTART;
-- @test
SELECT nextval('crst3');
