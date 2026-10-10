-- The body's own names are **gone** once the list renames them — the silent half of
-- the pre-close divergence, where `SELECT a FROM c` answered rows.
-- @expect-error: the body column name does not survive the rename
-- @setup
CREATE TABLE clg (a INT, b TEXT);
INSERT INTO clg VALUES (1, 'x');
-- @test
WITH c(p) AS (SELECT a FROM clg) SELECT a FROM c;
