-- A scalar subquery in a `WHERE`, whose type the clause scope answers through the
-- catalog — the shape the eval-time resolver cannot reach over zero rows.
-- @setup
CREATE TABLE bcn3 (a INT);
-- @expect-error: argument of WHERE must be type boolean
-- @test
SELECT count(*) FROM bcn3 WHERE (SELECT a FROM bcn3);
