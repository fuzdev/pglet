-- A window key resolves only the **first** entry it equals, and a key equal to no entry
-- (`'3'`, and `1`, which is a constant and not an ordinal here) resolves nothing: the
-- second `'2'` and `'4'`, which no key equals, take `integer`.
-- @setup
CREATE TABLE wdk (a INT);
INSERT INTO wdk VALUES (7);
-- @test
SELECT 'a' AS c, 1 AS v, 1 AS w, 1 AS x, 1 AS y
UNION ALL
SELECT '2', '2', row_number() OVER (PARTITION BY '2'), '4', row_number() OVER (PARTITION BY '3', 1) FROM wdk
ORDER BY 1;
