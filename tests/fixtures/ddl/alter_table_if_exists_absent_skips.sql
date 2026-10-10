-- Over a name no relation holds, `ALTER TABLE IF EXISTS` is a skip — a `NOTICE` and the
-- `ALTER TABLE` tag — whatever the action, before any analysis of it: a column type that
-- does not exist is never resolved. (The harness compares no notice; the host tests
-- pin it.)
-- @test
ALTER TABLE IF EXISTS nosuch_atie15 ADD COLUMN x nosuchtype_atie15;
