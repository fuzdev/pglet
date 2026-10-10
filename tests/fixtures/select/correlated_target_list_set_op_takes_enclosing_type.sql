-- The accept side of the set-operation member: the left side projects an enclosing
-- `INT` column over an inner table with a same-named `text` column, so the set
-- operation's column is an integer, `'9'` is `9`, and the sort is numeric — `2`, then
-- `9` where the outer `k` is `10`. Sorted as `text`, `'10'` comes before `'9'`.
-- @setup
CREATE TABLE ctu_te (k INT);
CREATE TABLE ctu_tt (m TEXT, k TEXT);
INSERT INTO ctu_te VALUES (2), (10);
INSERT INTO ctu_tt VALUES ('x', 'y');
-- @test
SELECT k, (SELECT o.k FROM ctu_tt UNION ALL SELECT '9' ORDER BY 1 LIMIT 1) AS v
  FROM ctu_te o ORDER BY k;
