-- A scalar subquery whose own column is a guess does not make the column it defines
-- certain, as the entry or as a branch of a fold. The sub-plan's fold is left at a guess
-- — its `double precision` branch, over a constructor's `unnest` subquery no reader
-- types, cast to the sibling's `integer` — and the cast the
-- pass writes over a branch subquery names that `integer` without the sub-plan vouching
-- for it. `avg` over either column stays `double precision`, 18.6's type for the
-- `double precision` column the fold is there; over a row that does not take the guessed
-- branch the value agrees too.
-- @setup
CREATE TABLE t (k INT, i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1, 1.5), (2, 2, 2.5);
-- @test
SELECT
  (SELECT avg(x) FROM
    (SELECT (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 WHERE t2.k = t.k) AS x FROM t) q)
    AS entry,
  (SELECT avg(x) FROM
    (SELECT COALESCE((SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 WHERE t2.k = t.k), 0) AS x
     FROM t) q) AS coalesce_branch,
  (SELECT avg(x) FROM
    (SELECT CASE WHEN k > 0 THEN (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 WHERE t2.k = t.k)
       ELSE 0 END AS x FROM t) q) AS case_branch,
  (SELECT avg(x) FROM
    (SELECT greatest((SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 WHERE t2.k = t.k), 0) AS x
     FROM t) q) AS greatest_arg,
  (SELECT avg(x) FROM (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) + 0 AS x FROM t) q) AS operand_fold;
