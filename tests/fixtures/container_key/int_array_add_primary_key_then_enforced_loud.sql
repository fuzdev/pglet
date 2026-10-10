-- `ALTER TABLE … ADD PRIMARY KEY` over an array column with rows in it.
-- @profile-lock
-- @setup
CREATE TABLE ckh_a (id INT, a INT[]);
INSERT INTO ckh_a (a, id) VALUES ('{1,2}', 1), ('{}', 2), ('{2}', 3);
ALTER TABLE ckh_a ADD PRIMARY KEY (a);
-- @test
-- @expect-error: a duplicate of a back-filled primary key (23505)
-- @expect-message
INSERT INTO ckh_a (a, id) VALUES ('{}', 4);
