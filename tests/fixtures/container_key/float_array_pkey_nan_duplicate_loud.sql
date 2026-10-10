-- Every `NaN` element is one key.
-- @profile-lock
-- @setup
CREATE TABLE ckn_f (id INT, a FLOAT8[] PRIMARY KEY);
INSERT INTO ckn_f (a, id) VALUES ('{NaN}', 1), ('{1.5}', 2);
-- @test
-- @expect-error: a float8[] NaN duplicate (23505)
-- @expect-message
INSERT INTO ckn_f (a, id) VALUES ('{NaN}', 3);
