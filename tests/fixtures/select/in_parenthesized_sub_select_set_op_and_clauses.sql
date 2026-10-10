-- A parenthesized query followed by a set operation or a clause over the whole query is
-- still the `IN` sub-select form.
-- @setup
CREATE TABLE r2 (a INT);
INSERT INTO r2 VALUES (1), (2);
-- @test
SELECT
  5 IN ((SELECT a FROM r2) UNION SELECT 5) AS union_bare,
  5 IN ((SELECT a FROM r2) UNION (SELECT 5)) AS union_parens,
  2 IN ((SELECT a FROM r2) ORDER BY 1 LIMIT 1) AS limited,
  2 NOT IN (((SELECT a FROM r2) EXCEPT SELECT 2)) AS except_nested;
