-- A derived table's column defined by an aggregate carries the aggregate's type:
-- `max` of a `bigint` is `bigint`, `count` is `bigint`, `sum` of an `integer` `bigint`,
-- so `avg` over each is the exact `numeric` and `sum` over each `numeric`. An aggregate
-- written without an alias is the column named for it.
-- @setup
CREATE TABLE o (id INT, a INT, b BIGINT);
INSERT INTO o VALUES (1, 1, 1), (2, 1, 4), (3, 2, 9007199254740993), (4, 2, 1);
-- @test
SELECT (SELECT avg(x) FROM (SELECT max(b) AS x FROM o GROUP BY id) v) AS avg_max,
  (SELECT round(avg(x)) FROM (SELECT max(b) AS x FROM o WHERE id < 3 GROUP BY id) v) AS round_avg,
  (SELECT sum(x) FROM (SELECT max(b) AS x FROM o GROUP BY a) v) AS sum_max,
  (SELECT sum(c) FROM (SELECT count(*) AS c FROM o GROUP BY a) v) AS sum_count,
  (SELECT avg(s) FROM (SELECT sum(a) AS s FROM o GROUP BY a) v) AS avg_sum,
  (SELECT avg(max) FROM (SELECT max(a) FROM o GROUP BY a) v) AS avg_unaliased;
