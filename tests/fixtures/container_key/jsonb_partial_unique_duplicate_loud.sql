-- A partial unique index over a bare `jsonb` column: the predicate decides membership,
-- so the same key repeats outside it and is a duplicate inside it.
-- @profile-lock
-- @setup
CREATE TABLE ckp_j (v TEXT, payload JSONB, tenant INT);
CREATE UNIQUE INDEX ckp_j_u ON ckp_j (payload) WHERE tenant > 100;
INSERT INTO ckp_j (tenant, v, payload) VALUES (1, 'a', '{"k":9}'), (2, 'b', '{"k":9}'), (200, 'c', '{"k":9}');
-- @test
-- @expect-error: a duplicate inside the partial predicate (23505)
-- @expect-message
INSERT INTO ckp_j (payload, v, tenant) VALUES ('{"k":9.0}', 'd', 201);
