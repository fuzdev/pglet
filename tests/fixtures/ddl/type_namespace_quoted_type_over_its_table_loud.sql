-- 18.6's own collision for a quoted name: an enum `"Tnt23"` over the table `"Tnt23"`
-- meets the table's row type. pglet stores the enum under its ASCII-folded key
-- (`tnt23`), which the table's name is not, so the name as written is asked too.
-- @setup
CREATE TABLE "Tnt23" (a INT);
-- @expect-error: a quoted enum over its table's exact name
-- @expect-message
-- @test
CREATE TYPE "Tnt23" AS ENUM ('x');
