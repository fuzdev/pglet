-- Non-unique indexes with a `jsonb` key part — a composite one whose `jsonb` part leads,
-- and an expression one — take every write to the table: insert, update of the key,
-- update of another column, delete.
-- @profile-lock
-- @setup
CREATE TABLE cki_j (v TEXT, payload JSONB, tenant INT);
CREATE INDEX cki_j_c ON cki_j (payload, v);
CREATE INDEX cki_j_e ON cki_j ((payload -> 'k'));
INSERT INTO cki_j (tenant, v, payload) VALUES (1, 'a', '{"k":1}'), (2, 'b', '{"k":1}'), (1, 'c', '{"k":[1,2]}'), (3, 'd', NULL);
UPDATE cki_j SET payload = '{"k":{"n":1}}' WHERE v = 'c';
UPDATE cki_j SET tenant = 9 WHERE v = 'b';
DELETE FROM cki_j WHERE v = 'a';
INSERT INTO cki_j (payload, v, tenant) VALUES ('{"k":1.0}', 'e', 1);
-- @test
SELECT v, tenant, payload::text AS key FROM cki_j ORDER BY v;
