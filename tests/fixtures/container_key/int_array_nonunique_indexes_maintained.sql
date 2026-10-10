-- Non-unique indexes with an array key part — a composite one whose array part leads,
-- and an expression one — take every write to the table.
-- @profile-lock
-- @setup
CREATE TABLE cki_a (v TEXT, a INT[], tenant INT);
CREATE INDEX cki_a_c ON cki_a (a, v);
CREATE INDEX cki_a_e ON cki_a ((ARRAY[tenant, tenant]));
INSERT INTO cki_a (tenant, v, a) VALUES (1, 'a', '{1,2}'), (2, 'b', '{1,2}'), (1, 'c', '{}'), (3, 'd', NULL);
UPDATE cki_a SET a = '{NULL,7}' WHERE v = 'c';
UPDATE cki_a SET tenant = 9 WHERE v = 'b';
DELETE FROM cki_a WHERE v = 'a';
INSERT INTO cki_a (a, v, tenant) VALUES ('{1,2}', 'e', NULL);
-- @test
SELECT v, tenant, a FROM cki_a ORDER BY v;
