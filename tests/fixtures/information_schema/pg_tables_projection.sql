-- pg_tables enumerates user tables. tableowner (pglet has no roles) and
-- hastriggers (Postgres installs FK system triggers, pglet has none) diverge, so
-- project only the stable schemaname / tablename columns.
-- @setup
CREATE TABLE pgt_parent (id INT PRIMARY KEY);
CREATE TABLE pgt_child (id INT PRIMARY KEY, parent_id INT REFERENCES pgt_parent(id));

-- @test
SELECT schemaname::text AS schemaname, tablename::text AS tablename
FROM pg_tables
WHERE schemaname = 'public' AND tablename LIKE 'pgt_%'
ORDER BY tablename;
