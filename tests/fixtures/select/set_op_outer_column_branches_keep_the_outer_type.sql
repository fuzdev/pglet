-- `CASE`, `ARRAY[…]`, `nullif` and `least` over a set operation naming the enclosing
-- `numeric` column each take `numeric` beside an `integer` constant.
-- @setup
CREATE TABLE sobk (n NUMERIC);
INSERT INTO sobk VALUES (1.25);
-- @test
SELECT CASE WHEN true THEN (SELECT n EXCEPT SELECT 1) ELSE 2 END AS c,
       ARRAY[(SELECT n EXCEPT SELECT 1), 2] AS a,
       nullif((SELECT n EXCEPT SELECT 1), 1) AS ni,
       least((SELECT n EXCEPT SELECT 1), 9) AS l
FROM sobk;
