-- Accept-side controls, both directions: an index carries no row type and
-- `index_create` never asks the type namespace, and a sequence carries none either, so
-- an index over an enum's name and an enum over an index's or a sequence's name are all
-- accepted — the setup's own statements are the assertion, and the `@test` reads each
-- enum back through its name.
-- @setup
CREATE TABLE tnt21t (a INT);
CREATE TYPE tnt21i AS ENUM ('x');
CREATE INDEX tnt21i ON tnt21t (a);
CREATE INDEX tnt21j ON tnt21t (a);
CREATE TYPE tnt21j AS ENUM ('y');
CREATE SEQUENCE tnt21s;
CREATE TYPE tnt21s AS ENUM ('z');
SELECT nextval('tnt21s');
-- @test
SELECT 'x'::tnt21i::text AS i, 'y'::tnt21j::text AS j, 'z'::tnt21s::text AS s;
