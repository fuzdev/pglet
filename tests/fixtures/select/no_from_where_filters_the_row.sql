-- A select with no FROM reads one row, and a `WHERE` keeps it or drops it: true, false and
-- NULL, a correlated `EXISTS` beside it, and set-operation operands each filtered on their
-- own.
-- @setup
CREATE TABLE nfw (a int);
INSERT INTO nfw VALUES (1), (3);
-- @test
SELECT 1 AS x WHERE true
UNION ALL SELECT 2 WHERE false
UNION ALL SELECT 3 WHERE NULL
UNION ALL SELECT 4 WHERE EXISTS (SELECT 1 FROM nfw WHERE a = 3)
UNION ALL SELECT 5 WHERE 'a' = $$a$$
ORDER BY x;
