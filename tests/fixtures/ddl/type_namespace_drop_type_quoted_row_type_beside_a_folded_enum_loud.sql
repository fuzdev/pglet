-- `DROP TYPE "Tnt30"` beside a table `"Tnt30"` and an enum `tnt30` names the table's row
-- type, which is refused through the table's internal dependency (`2BP01`); the enum the
-- name's folded key finds is not what the statement names. pglet read the folded key
-- first and dropped the enum. The message quotes the mixed-case name, as 18.6's does.
-- @setup
CREATE TYPE tnt30 AS ENUM ('a');
CREATE TABLE "Tnt30" (a INT);
-- @expect-error: DROP TYPE of a quoted table's row type
-- @expect-message
-- @test
DROP TYPE "Tnt30";
