-- A row the upsert inserts into the parent supplies a substitute key for the parent's own
-- foreign keys only: a grandchild of the cascaded child still refuses the move, although
-- the inserted parent row holds the vacated value in the column at the same position.
-- @profile-lock
-- @setup
CREATE TABLE upto_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upto_c (id INT PRIMARY KEY, pid INT UNIQUE REFERENCES upto_p(id) ON UPDATE CASCADE);
CREATE TABLE upto_g (cp INT REFERENCES upto_c(pid));
INSERT INTO upto_p VALUES (1, 0);
INSERT INTO upto_c VALUES (10, 1);
INSERT INTO upto_g VALUES (1);

-- @test
-- @expect-error: the cascaded upto_c.pid 1 is still referenced by upto_g (23503)
INSERT INTO upto_p VALUES (1, 0), (5, 1) ON CONFLICT (id) DO UPDATE SET id = 9;
