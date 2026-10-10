-- A `DROP` frees the name for the other kind, in both directions.
-- @setup
CREATE TYPE tnt22a AS ENUM ('x');
DROP TYPE tnt22a;
CREATE TABLE tnt22a (a INT);
INSERT INTO tnt22a VALUES (1);
CREATE TABLE tnt22b (a INT);
DROP TABLE tnt22b;
CREATE TYPE tnt22b AS ENUM ('y');
-- @test
SELECT (SELECT a FROM tnt22a) AS a, 'y'::tnt22b::text AS b;
