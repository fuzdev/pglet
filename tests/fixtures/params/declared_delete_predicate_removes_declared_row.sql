-- It writes: a `DELETE` whose predicate compares against `coalesce($1, 0)` with `$1`
-- declared `numeric` and bound `1.5` removes the `1.5` row. Converting the parameter to
-- the literal's `integer` compared against `2` and removed that row instead.
-- @setup
CREATE TABLE dp_del (n NUMERIC);
INSERT INTO dp_del VALUES (1.5), (2);
-- @param-declared $1 numeric 1.5
-- @test
DELETE FROM dp_del WHERE n = coalesce($1, 0) RETURNING n;
