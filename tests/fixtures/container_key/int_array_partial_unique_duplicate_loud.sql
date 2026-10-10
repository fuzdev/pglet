-- A partial unique index over a bare array column: the same key repeats outside the
-- predicate and is a duplicate inside it.
-- @profile-lock
-- @setup
CREATE TABLE ckp_a (v TEXT, a INT[], tenant INT);
CREATE UNIQUE INDEX ckp_a_u ON ckp_a (a) WHERE tenant > 100;
INSERT INTO ckp_a (tenant, v, a) VALUES (1, 'a', '{9,NULL}'), (2, 'b', '{9,NULL}'), (200, 'c', '{9,NULL}');
-- @test
-- @expect-error: a duplicate inside the partial predicate (23505)
-- @expect-message
INSERT INTO ckp_a (a, v, tenant) VALUES ('{9,NULL}', 'd', 201);
