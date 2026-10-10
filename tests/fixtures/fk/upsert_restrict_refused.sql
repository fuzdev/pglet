-- An `ON CONFLICT DO UPDATE` that moves a key an `ON UPDATE RESTRICT` child references is
-- `23001`, not the `NO ACTION` `23503`.
-- @profile-lock
-- @setup
CREATE TABLE uprs_p (id INT PRIMARY KEY, v INT);
CREATE TABLE uprs_c (pid INT REFERENCES uprs_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO uprs_p VALUES (1, 1), (2, 2);
INSERT INTO uprs_c VALUES (1, 10);

-- @test
-- @expect-error: RESTRICT refuses the rewrite of key 1 (23001)
INSERT INTO uprs_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9;
