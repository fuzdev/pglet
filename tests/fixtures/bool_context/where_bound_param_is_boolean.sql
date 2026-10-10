-- The spine's real binding path: an untyped `$N` at a boolean position is
-- `boolean`, as `coerce_to_boolean` resolves an unknown Param there. Reported
-- OID 0 here, which decoded the bind as text and matched nothing.
-- @setup
CREATE TABLE bcp (a INT);
INSERT INTO bcp VALUES (1);
-- @param $1 bool true
-- @test
SELECT count(*) FROM bcp WHERE $1;
