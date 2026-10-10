-- The composite key of `jsonb_composite_unique_duplicate_loud.sql` through its writes:
-- an update moves a row's key, a delete frees one, and the freed key is insertable again.
-- @profile-lock
-- @setup
CREATE TABLE ckw_j (v TEXT, payload JSONB, tenant INT, UNIQUE (tenant, payload));
INSERT INTO ckw_j (tenant, v, payload) VALUES (1, 'a', '{"k":1}'), (2, 'b', '{"k":1}'), (1, 'c', '{"k":2}');
UPDATE ckw_j SET payload = '{"k":3}' WHERE v = 'c';
DELETE FROM ckw_j WHERE v = 'a';
INSERT INTO ckw_j (payload, v, tenant) VALUES ('{"k":1.0}', 'd', 1), ('{"k":2.0}', 'e', 1);
INSERT INTO ckw_j (payload, v, tenant) VALUES ('{"k":3.0}', 'f', 1) ON CONFLICT (tenant, payload) DO UPDATE SET v = 'c2';
-- @test
SELECT v, tenant, payload::text AS key FROM ckw_j ORDER BY v;
