-- @setup
CREATE TABLE ra (a INT, b INT, c INT);
INSERT INTO ra VALUES (10, 400, 1), (20, 300, 2), (30, 200, 3), (40, 100, 4);

-- `b AS a` makes the output column name `a` shadow the *base* column `a`, and `b`
-- runs opposite to `a` so the two orderings are opposite. Postgres's SQL92 rule
-- gives the unqualified whole key `a` to the output column, so this sorts by `b`
-- (100, 200, 300, 400); the trailing `c` is dropped from the select list, which
-- forces the Sort below the projection. Down there the key must be rewritten to the
-- projection expression `b` — reading the same-named base column `a` instead sorts
-- the rows exactly backwards, silently.
-- @test
SELECT b AS a FROM ra ORDER BY a, c;
