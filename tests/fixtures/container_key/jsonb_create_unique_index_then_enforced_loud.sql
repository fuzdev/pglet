-- `CREATE UNIQUE INDEX` over a bare `jsonb` column with distinct rows in it builds, and
-- then refuses a duplicate of a row that was there first.
-- @profile-lock
-- @setup
CREATE TABLE cki_u (id INT, j JSONB);
INSERT INTO cki_u (j, id) VALUES ('{"a":1}', 1), ('{"a":2}', 2), (NULL, 3), (NULL, 4);
CREATE UNIQUE INDEX cki_u_ix ON cki_u (j);
-- @test
-- @expect-error: a duplicate of a back-filled key (23505)
-- @expect-message
INSERT INTO cki_u (j, id) VALUES ('{"a":2.0}', 5);
