-- A `float8` value stored into a `real` key rounds to the key another row holds.
-- @profile-lock
-- @expect-error: the stored value is the real 0.1 (23505)
-- @expect-message
-- @setup
CREATE TABLE fkp (b TEXT, x REAL PRIMARY KEY, a INT);
INSERT INTO fkp VALUES ('c', 0.1, 3);
-- @test
INSERT INTO fkp VALUES ('d', 0.1::float8, 4);
