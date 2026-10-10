-- The unquoted spelling beside the same pair names the enum on both engines, and drops
-- it; the table `"Tnt31"` and its row stay. The control for the quoted refusal: the
-- row-type lookup reads the name as written, so it does not reach this enum.
-- @setup
CREATE TYPE tnt31 AS ENUM ('a');
CREATE TABLE "Tnt31" (a INT);
INSERT INTO "Tnt31" VALUES (1);
DROP TYPE tnt31;
-- @test
SELECT (SELECT count(*) FROM pg_type WHERE typname = 'tnt31') AS enums,
       (SELECT a FROM "Tnt31") AS a;
