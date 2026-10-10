-- The alias-qualified spelling of an enclosing query's column, which names no table the
-- catalog holds, carries its `integer` too.
-- @setup
CREATE TABLE dsa (i INT);
INSERT INTO dsa VALUES (7);
-- @param-declared $1 int4 5
-- @test
SELECT count(*) FROM dsa t WHERE t.i IN (SELECT $1 UNION SELECT t.i);
