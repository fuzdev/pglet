-- A set-operation side projecting the enclosing query's column carries that column's
-- `integer` into the unification, so it matches the declared `integer` parameter.
-- @setup
CREATE TABLE dso (i INT);
INSERT INTO dso VALUES (7);
-- @param-declared $1 int4 5
-- @test
SELECT count(*) FROM dso WHERE EXISTS (SELECT $1 UNION SELECT i);
