-- `CREATE UNIQUE INDEX` over a bare array column with distinct rows in it builds, and
-- then refuses a duplicate of a row that was there first.
-- @profile-lock
-- @setup
CREATE TABLE cki_v (id INT, a INT[]);
INSERT INTO cki_v (a, id) VALUES ('{1,2}', 1), ('{2,1}', 2), (NULL, 3), (NULL, 4);
CREATE UNIQUE INDEX cki_v_ix ON cki_v (a);
-- @test
-- @expect-error: a duplicate of a back-filled key (23505)
-- @expect-message
INSERT INTO cki_v (a, id) VALUES ('{2,1}', 5);
