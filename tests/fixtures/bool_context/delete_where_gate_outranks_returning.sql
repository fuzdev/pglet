-- …and the `RETURNING` clause, on the `DELETE` route.
-- @setup
CREATE TABLE bcord6 (k INT);
-- @expect-error: argument of WHERE must be type boolean, not RETURNING's 42703
-- @test
DELETE FROM bcord6 WHERE k RETURNING nosuchcol;
