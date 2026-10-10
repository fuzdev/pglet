-- pg_indexes lists every index, including the implicit PK (<table>_pkey) and
-- UNIQUE-constraint (<table>_<col>_key) indexes, with the explicit ones. The
-- indexdef / tablespace columns diverge (best-effort / NULL), so project the
-- stable schemaname / tablename / indexname.
-- @setup
CREATE TABLE pgi_t (id INT PRIMARY KEY, email TEXT UNIQUE, name TEXT);
CREATE INDEX idx_pgi_name ON pgi_t(name);

-- @test
SELECT schemaname::text AS schemaname, tablename::text AS tablename, indexname::text AS indexname
FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'pgi_t'
ORDER BY indexname;
