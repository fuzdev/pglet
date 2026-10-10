-- A derived table's column defined by a fold the coercion pass leaves at a guess — a
-- branch over a scalar subquery whose own column is one (a constructor's `unnest`),
-- cast to its sibling's `integer` — is not of a certain
-- type, though the resolved fold reads as typed: `avg` over it stays `double precision`,
-- which is 18.6's type for the `double precision` column the fold is there. Over rows
-- that never take the guessed branch the value agrees too. The same holds with the fold
-- as either side of a set operation.
-- @setup
CREATE TABLE t (i INT, f FLOAT8);
INSERT INTO t VALUES (1, 1.5), (2, 2.5), (5, NULL);
-- @test
SELECT (SELECT avg(x) FROM (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) AS x FROM t) q) AS derived,
  (SELECT avg(x) FROM
    (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) AS x FROM t UNION ALL SELECT i FROM t) q) AS left_side,
  (SELECT avg(x) FROM
    (SELECT i AS x FROM t UNION ALL SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t) q) AS right_side;
