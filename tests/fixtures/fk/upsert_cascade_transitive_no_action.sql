-- A key an upsert cascades into a child is itself referenced by a grandchild whose
-- `NO ACTION` foreign key refuses the move.
-- @profile-lock
-- @setup
CREATE TABLE upct_p (id INT PRIMARY KEY);
CREATE TABLE upct_c (id INT PRIMARY KEY, pid INT UNIQUE REFERENCES upct_p(id) ON UPDATE CASCADE);
CREATE TABLE upct_g (cp INT REFERENCES upct_c(pid));
INSERT INTO upct_p VALUES (1), (2);
INSERT INTO upct_c VALUES (10, 1);
INSERT INTO upct_g VALUES (1);

-- @test
-- @expect-error: the cascaded upct_c.pid is still referenced by upct_g (23503)
INSERT INTO upct_p VALUES (1) ON CONFLICT (id) DO UPDATE SET id = 9;
