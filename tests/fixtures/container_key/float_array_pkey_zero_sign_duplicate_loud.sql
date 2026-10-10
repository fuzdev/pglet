-- A `float8[]` key takes each element's float key: `-0` is `0`.
-- @profile-lock
-- @setup
CREATE TABLE ckz_f (id INT, a FLOAT8[] PRIMARY KEY);
INSERT INTO ckz_f (a, id) VALUES ('{0}', 1), ('{1.5}', 2);
-- @test
-- @expect-error: a float8[] duplicate across the sign of zero (23505)
-- @expect-message
INSERT INTO ckz_f (a, id) VALUES ('{-0}', 3);
