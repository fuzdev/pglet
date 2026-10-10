-- @setup
CREATE TABLE gj (k INT, v INT);
INSERT INTO gj VALUES (1, 10), (2, 20), (2, 21), (3, 30), (3, 31), (3, 32);

-- Only a **nested** query level re-resolves its own expressions against the
-- chained scope, and that walk used to re-gate an `ORDER BY` key against the FROM
-- relations alone — where a key naming an output column, or a junk column, binds to
-- nothing. Both spellings are here: the outer subquery's key is an output alias, the
-- inner one's is a grouping column the select list drops.
-- @test
SELECT
  (SELECT count(*) AS c FROM gj GROUP BY k ORDER BY c DESC LIMIT 1) AS by_alias,
  (SELECT count(*) FROM gj GROUP BY k ORDER BY k DESC LIMIT 1) AS by_junk;
