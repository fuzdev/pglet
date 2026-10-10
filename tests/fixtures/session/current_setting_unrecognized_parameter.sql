-- The same `42704` `SHOW nosuchthing` raises, from the same table — one decider,
-- not two.
-- @expect-error: unrecognized configuration parameter
-- @test
SELECT current_setting('nosuchthing');
