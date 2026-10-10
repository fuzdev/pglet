-- A trailing ORDER BY / LIMIT binds to the whole combined result, and a positional
-- `ORDER BY 1` references the first output column. Sorted descending by `a` with
-- LIMIT 2 keeps the two largest distinct rows.
-- @setup
CREATE TABLE sob (a INT, b TEXT);
INSERT INTO sob VALUES (3, 'x'), (1, 'y'), (2, 'z');
-- @test
SELECT a, b FROM sob
UNION
SELECT a, b FROM sob
ORDER BY 1 DESC
LIMIT 2;
