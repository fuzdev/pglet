-- A `DELETE … RETURNING` whose expression is ill-typed must fail when the statement is
-- planned, not when a row happens to reach it. The `WHERE` here matches **nothing**, so
-- the projection never evaluates and the eval-time gate never fires — pglet reported
-- `DELETE 0` and no error where Postgres rejects the statement outright.
-- @setup
CREATE TABLE dro (id INT, i4 INT4);
INSERT INTO dro VALUES (1, 10);
-- @expect-error: integer LIKE text has no operator in RETURNING (42883)
-- @test
DELETE FROM dro WHERE id = 999 RETURNING i4 LIKE 'x%';
