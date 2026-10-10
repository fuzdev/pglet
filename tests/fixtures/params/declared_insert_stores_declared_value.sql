-- It writes: an `INSERT` of `coalesce($1, 1)` with `$1` declared `numeric` stores `1.5`,
-- not the `2` a conversion to the literal's `integer` stored.
-- @setup
CREATE TABLE dp_ins (n NUMERIC);
-- @param-declared $1 numeric 1.5
-- @test
INSERT INTO dp_ins VALUES (coalesce($1, 1)) RETURNING n;
