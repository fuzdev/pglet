-- A column's value converts per row: over an empty table nothing is converted, on both
-- engines.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE e (s TEXT);

-- @test
SELECT s::mood::text AS v FROM e;
