-- A CTE shadows a same-named base table for the whole statement, so the list names the
-- columns a reader sees under that name — the table's own columns are not in scope.
-- @setup
CREATE TABLE clsh (a INT, b TEXT);
INSERT INTO clsh VALUES (1, 'x');
-- @test
WITH clsh(z) AS (SELECT 9) SELECT z FROM clsh;
