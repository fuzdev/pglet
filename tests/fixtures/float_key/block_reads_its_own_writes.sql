-- Inside a block the index holds none of the block's rows yet, so a probe merges them
-- from the pending writes: equality, range and a NaN, each as the committed index would
-- answer it.
-- @profile-lock
-- @setup
CREATE TABLE fkrw (k INT, f FLOAT8, g REAL NOT NULL);
CREATE INDEX fkrw_f ON fkrw (f);
CREATE INDEX fkrw_g ON fkrw (g);
INSERT INTO fkrw VALUES (1, 1.5, 4), (2, 'NaN', 3), (3, '-0', 2), (4, -7, 1);
UPDATE fkrw SET f = 9, g = 'NaN' WHERE k = 4;
DELETE FROM fkrw WHERE f = 1.5;
-- @test
SELECT 'eq' AS probe, k FROM fkrw WHERE f = 0
UNION ALL
SELECT 'range', k FROM fkrw WHERE f > 5
UNION ALL
SELECT 'gone', k FROM fkrw WHERE f = 1.5
UNION ALL
SELECT 'gone', k FROM fkrw WHERE f = -7
UNION ALL
SELECT 'order', k FROM (SELECT k FROM fkrw ORDER BY g DESC LIMIT 1) top
ORDER BY 1, 2;
