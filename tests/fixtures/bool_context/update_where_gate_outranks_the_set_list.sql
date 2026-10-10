-- `transformUpdateStmt` runs FROM, `WHERE`, `RETURNING` and only then the target
-- list, so the `WHERE`'s gate beats an unbindable `SET` target.
-- @setup
CREATE TABLE bcord5 (k INT);
-- @expect-error: argument of WHERE must be type boolean, not the SET list's 42703
-- @test
UPDATE bcord5 SET nosuch = 1 WHERE k;
