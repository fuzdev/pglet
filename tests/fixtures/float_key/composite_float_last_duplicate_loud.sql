-- The float is the trailing part of the key and precedes the other part in the table.
-- @profile-lock
-- @expect-error: (1, -0) = (1, 0) (23505)
-- @expect-message
-- @setup
CREATE TABLE fkn (r REAL, note TEXT, a INT, UNIQUE (a, r));
INSERT INTO fkn VALUES ('NaN', 'x', 1), (0, 'y', 1);
-- @test
INSERT INTO fkn VALUES ('-0', 'z', 1);
