-- A parenthesized scalar subquery with anything more in the `IN` body — another item, an
-- operator, a cast — makes the body an expression list, and the scalar subquery over an
-- empty table is a NULL item.
-- @setup
CREATE TABLE e (a INT);
-- @test
SELECT
  1 IN ((SELECT a FROM e), 5) AS beside_item,
  1 IN ((SELECT a FROM e) + 0) AS under_operator,
  1 IN (((SELECT a FROM e)) + 0) AS doubled_under_operator,
  1 IN (((SELECT a FROM e) + 0)) AS operator_in_parens,
  1 IN ((SELECT a FROM e)::int) AS under_cast,
  1 IN ((1), 2) AS paren_item,
  1 IN ((1)) AS doubled_item;
