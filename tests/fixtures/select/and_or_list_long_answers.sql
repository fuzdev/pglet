-- `AND` and `OR` are not bounded by the depth limit: 18.6 flattens either into one
-- n-ary node, and pglet builds a list longer than 64 operands balanced, so both
-- answer a list of any length. 400-operand lists, each connective and nested.
-- @setup
CREATE TABLE al (a INT, b INT);
INSERT INTO al VALUES (1, 1), (1, 2), (2, 1), (3, 3);
-- @test
SELECT
  (SELECT count(*) FROM al WHERE a = 1
     AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0
 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0 AND b > 0) AS all_and,
  (SELECT count(*) FROM al WHERE a = 9
     OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2
 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2 OR b = 2) AS all_or,
  (SELECT count(*) FROM al WHERE (a = 1
     OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3
 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3 OR a = 3)
    AND (b = 1
     OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3
 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3 OR b = 3)) AS nested;
