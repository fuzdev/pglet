-- A composite unique key with a `jsonb` part — `UNIQUE (tenant, payload)`, the key's
-- column order not the table's, the `INSERT` column list in a third order. The same
-- payload under another tenant is a distinct key; under the same tenant, at another
-- scale, a duplicate.
-- @profile-lock
-- @setup
CREATE TABLE ckm_j (v TEXT, payload JSONB, tenant INT, UNIQUE (tenant, payload));
INSERT INTO ckm_j (tenant, v, payload) VALUES (1, 'a', '{"k":1}'), (2, 'b', '{"k":1}'), (1, 'c', '{"k":2}'), (1, 'd', NULL), (1, 'e', NULL);
-- @test
-- @expect-error: a composite duplicate through the jsonb part (23505)
-- @expect-message
INSERT INTO ckm_j (payload, v, tenant) VALUES ('{"k":1.0}', 'f', 1);
