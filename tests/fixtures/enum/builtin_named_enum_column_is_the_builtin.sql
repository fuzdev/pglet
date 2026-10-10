-- A column declared `int4` beside an enum named `int4` is an `integer` column: the type
-- name resolves as at a cast.
-- @setup
CREATE TYPE int4 AS ENUM ('k');
CREATE TABLE ci (c int4);
INSERT INTO ci VALUES ('5');

-- @test
SELECT c, c + 1 AS n FROM ci;
