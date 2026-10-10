-- A composite unique key with an array part, `UNIQUE (tenant, tags)`, through its
-- writes: the same tags under another tenant are distinct, an update moves a key, a
-- delete frees one.
-- @profile-lock
-- @setup
CREATE TABLE ckw_t (v TEXT, tags TEXT[], tenant INT, UNIQUE (tenant, tags));
INSERT INTO ckw_t (tenant, v, tags) VALUES (1, 'a', '{x,y}'), (2, 'b', '{x,y}'), (1, 'c', '{x}');
UPDATE ckw_t SET tags = '{x,y,z}' WHERE v = 'c';
DELETE FROM ckw_t WHERE v = 'a';
INSERT INTO ckw_t (tags, v, tenant) VALUES ('{x,y}', 'd', 1);
INSERT INTO ckw_t (tags, v, tenant) VALUES ('{x,y,z}', 'e', 1) ON CONFLICT (tenant, tags) DO UPDATE SET v = 'c2';
-- @test
SELECT v, tenant, tags FROM ckw_t ORDER BY v;
