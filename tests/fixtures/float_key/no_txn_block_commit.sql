-- @no-txn
-- A block's writes to a `float8` and a `real` key are keyed when it commits. The
-- block's own probe reads them before that, and the committed index holds them after.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS fknb CASCADE;
CREATE TABLE fknb (k INT, f FLOAT8, r REAL);
CREATE INDEX fknb_f ON fknb (f);
CREATE UNIQUE INDEX fknb_r ON fknb (r);
BEGIN;
INSERT INTO fknb VALUES (1, 1.5, 1.5), (2, 'NaN', '-0');
UPDATE fknb SET f = 2.5 WHERE f = 1.5;
COMMIT;
BEGIN;
INSERT INTO fknb VALUES (3, 3.5, 3.5);
ROLLBACK;
-- @test
SELECT 'f' AS probe, k FROM fknb WHERE f >= 2.5
UNION ALL
SELECT 'r', k FROM fknb WHERE r = 0
UNION ALL
SELECT 'gone', k FROM fknb WHERE r = 3.5
ORDER BY 1, 2;
