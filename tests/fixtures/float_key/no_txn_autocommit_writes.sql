-- @no-txn
-- Autocommit maintenance of a float key, outside the harness's transaction wrapper:
-- each write keys its row as its own statement commits, and the probes read the
-- committed index.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS fkna CASCADE;
CREATE TABLE fkna (k INT, r REAL, f FLOAT8);
CREATE INDEX fkna_f ON fkna (f);
CREATE UNIQUE INDEX fkna_rk ON fkna (r, k);
INSERT INTO fkna VALUES (1, 1.5, 1.5);
INSERT INTO fkna VALUES (2, 2.5, 'NaN'), (3, 'NaN', '-0');
UPDATE fkna SET f = 2.5, r = 0 WHERE k = 1;
INSERT INTO fkna VALUES (4, 1.5, 2.5);
DELETE FROM fkna WHERE f = 0;
-- @test
SELECT 'eq' AS probe, k FROM fkna WHERE f = 2.5
UNION ALL
SELECT 'nan', k FROM fkna WHERE f > 1e308
UNION ALL
SELECT 'old', k FROM fkna WHERE f = 1.5
UNION ALL
SELECT 'zero', k FROM fkna WHERE f = 0
ORDER BY 1, 2;
