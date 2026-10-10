-- `IF EXISTS` skips only a name no type holds, and `CASCADE` does not reach an internal
-- dependency: both are `2BP01` over a table's row type. Before the row type was modelled
-- this spelling reported `DROP TYPE` and did nothing — the silent member.
-- @setup
CREATE TABLE tnt20 (a INT);
-- @expect-error: DROP TYPE IF EXISTS of a table's row type
-- @expect-message
-- @test
DROP TYPE IF EXISTS tnt20 CASCADE;
