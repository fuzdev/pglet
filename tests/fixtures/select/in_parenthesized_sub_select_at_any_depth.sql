-- An `IN` body that is a parenthesized sub-select, at any depth of parentheses, is the
-- `IN` sub-select form: over an empty sub-select `IN` is `false` and `NOT IN` `true`, and a
-- sub-select of two rows is a set, not a scalar subquery.
-- @setup
CREATE TABLE e (a INT);
CREATE TABLE r2 (a INT);
INSERT INTO r2 VALUES (1), (2);
-- @test
SELECT
  1 IN ((SELECT a FROM e)) AS in_empty,
  1 NOT IN ((SELECT a FROM e)) AS not_in_empty,
  NULL::int NOT IN ((SELECT a FROM e)) AS null_not_in_empty,
  1 IN (((SELECT a FROM r2))) AS in_two_rows,
  3 IN ((((SELECT a FROM r2)))) AS not_in_two_rows,
  NOT (1 IN ((SELECT a FROM e))) AS not_of_in_empty;
