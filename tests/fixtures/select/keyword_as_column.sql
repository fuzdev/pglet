-- @setup
-- Non-reserved keywords (`sequence`, `name`) used as ordinary column names, as
-- the spine's migration runner does (`SELECT name, sequence FROM schema_version
-- ORDER BY sequence ASC`). Postgres accepts a non-reserved keyword in expression
-- position; pglet must too.
CREATE TABLE schema_version (name TEXT, sequence INTEGER);
INSERT INTO schema_version VALUES ('b', 2), ('a', 1);

-- @test
SELECT name, sequence FROM schema_version WHERE sequence > 0 ORDER BY sequence ASC;
