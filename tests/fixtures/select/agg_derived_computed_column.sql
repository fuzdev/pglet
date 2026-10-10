-- A derived table's column defined by an expression the coercion pass types as written
-- — arithmetic over a table's columns, a cast, a function — is of a certain type, so
-- `avg` over it is the exact `numeric`, `sum` of a `bigint` one the exact `numeric`
-- total, and a function over the average reads a `numeric`.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT, tx TEXT);
INSERT INTO t VALUES (1, 1, 1, 'a'), (2, 4, 9007199254740993, 'bbbb'), (3, NULL, NULL, NULL),
  (4, 2, 2, 'cc');
-- @test
SELECT avg(x) AS a, sum(x) AS s, round(avg(y)) AS r, round(avg(y), 1) AS r1, avg(z) AS az,
  sum(w) AS sw, avg(l) AS al
FROM (SELECT b + 0 AS x, i * 1 AS y, i::bigint AS z, -b AS w, length(tx) AS l FROM t WHERE k <> 4) q;
