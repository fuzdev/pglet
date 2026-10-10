-- `ON CONFLICT (x)` arbitrates on the float key: `-0` conflicts with the stored `0`
-- and a NaN with the stored NaN, each taking the `DO UPDATE` arm.
-- @profile-lock
-- @setup
CREATE TABLE fkc (n INT, x FLOAT8 PRIMARY KEY);
INSERT INTO fkc VALUES (1, 0), (2, 'NaN'), (3, 1.5);
INSERT INTO fkc VALUES (10, '-0') ON CONFLICT (x) DO UPDATE SET n = excluded.n;
INSERT INTO fkc VALUES (20, 'NaN') ON CONFLICT (x) DO UPDATE SET n = fkc.n + excluded.n;
INSERT INTO fkc VALUES (30, 1.5) ON CONFLICT (x) DO NOTHING;
INSERT INTO fkc VALUES (40, 2.5) ON CONFLICT DO NOTHING;
-- @test
SELECT n, x::text AS shown FROM fkc ORDER BY x;
