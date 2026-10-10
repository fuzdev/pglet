-- A name no relation holds at all is the only one that reaches
-- `DropErrorMsgNonExistent` — `sequence "x" does not exist`.
-- @setup
CREATE TABLE dsm_t (a INT);
-- @test
DROP SEQUENCE dsm_nope;
-- @expect-error: no such sequence (42P01)
