-- A table's row type is *in* the type namespace, so `DROP TYPE <a table>` finds it and
-- is refused through the table's internal dependency (`2BP01`) — where pglet, not
-- modelling the row type, answered "does not exist".
-- @setup
CREATE TABLE tnt19 (a INT);
-- @expect-error: DROP TYPE of a table's row type
-- @expect-message
-- @test
DROP TYPE tnt19;
