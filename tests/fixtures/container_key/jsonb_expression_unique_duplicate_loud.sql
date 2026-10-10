-- A unique expression index whose key is a `jsonb` value: `(payload -> 'k')` is `1` for
-- both rows, at two scales.
-- @profile-lock
-- @setup
CREATE TABLE ckx_j (tenant INT, payload JSONB);
CREATE UNIQUE INDEX ckx_j_k ON ckx_j ((payload -> 'k'));
INSERT INTO ckx_j (payload, tenant) VALUES ('{"k":1,"z":1}', 1), ('{"z":2}', 2), ('{"k":[1]}', 3);
-- @test
-- @expect-error: a duplicate jsonb expression key (23505)
-- @expect-message
INSERT INTO ckx_j (payload, tenant) VALUES ('{"k":1.0,"z":3}', 4);
