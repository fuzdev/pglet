-- `IF NOT EXISTS` skips on the **relation** namespace only (`transformCreateStmt`), so
-- over an enum of the name the statement goes on and meets the type namespace's
-- `42710` — an error, not a skip.
-- @setup
CREATE TYPE tnt2 AS ENUM ('x');
-- @expect-error: IF NOT EXISTS does not skip over a type
-- @expect-message
-- @test
CREATE TABLE IF NOT EXISTS tnt2 (a INT);
