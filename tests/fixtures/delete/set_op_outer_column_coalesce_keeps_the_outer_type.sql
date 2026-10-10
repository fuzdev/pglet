-- It writes: `SELECT n EXCEPT SELECT 1` names the enclosing `numeric` column, so the scalar
-- subquery is a `numeric` column and `COALESCE` over it and `2` is `numeric`. The row
-- `1.25` matches `1.25`; the row `1` finds `1 EXCEPT 1` empty, takes `2`, and stays.
-- @setup
CREATE TABLE sock (n NUMERIC);
INSERT INTO sock VALUES (1.25), (1);
-- @test
DELETE FROM sock WHERE coalesce((SELECT n EXCEPT SELECT 1), 2) = 1.25 RETURNING n;
