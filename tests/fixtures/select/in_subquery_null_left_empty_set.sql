-- A NULL left operand of `IN (SELECT …)` over a sub-plan that returns no row is `false`,
-- and `NOT IN` is `true`: no comparison is made, so the operand's NULL never reaches one.
-- The `NOT (… IN …)` spelling takes the `IN` answer and negates it.
-- @setup
CREATE TABLE e (a INT);
-- @test
SELECT
  NULL::int IN (SELECT a FROM e) AS in_empty,
  NULL::int NOT IN (SELECT a FROM e) AS not_in_empty,
  NOT (NULL::int IN (SELECT a FROM e)) AS not_of_in_empty,
  (NULL::int IN (SELECT a FROM e)) IS FALSE AS in_empty_is_false;
