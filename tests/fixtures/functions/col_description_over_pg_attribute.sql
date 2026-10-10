-- The shape kysely's `PostgresIntrospector.getTables` issues: `col_description` over
-- `pg_attribute`'s own `attrelid` (an `oid`) and `attnum` (an `int2`), one row per column.
-- @setup
CREATE TABLE cd_people (id INT, name TEXT);
-- @test
SELECT a.attname, col_description(a.attrelid, a.attnum) AS column_description
FROM pg_catalog.pg_attribute AS a
JOIN pg_catalog.pg_class AS c ON a.attrelid = c.oid
WHERE c.relname = 'cd_people' AND a.attnum > 0
ORDER BY a.attnum;
