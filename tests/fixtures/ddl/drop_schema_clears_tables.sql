-- `DROP SCHEMA public CASCADE` removes every table in the schema, so a query
-- against a table that existed before the drop fails loud (42P01) afterward —
-- proving the cascade actually cleared the catalog, not just succeeded.
-- @expect-error: table removed by DROP SCHEMA CASCADE
-- @setup
CREATE TABLE gone (id INT);
INSERT INTO gone VALUES (1);
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
-- @test
SELECT * FROM gone;
