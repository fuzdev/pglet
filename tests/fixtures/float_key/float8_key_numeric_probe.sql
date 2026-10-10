-- A `numeric` probe of a `float8` key compares as `float8` on 18.6, so a value that
-- rounds to the key matches it — and the probe is served at the rounded value.
-- @profile-lock
-- @setup
CREATE TABLE fkw (k INT, x FLOAT8);
CREATE INDEX fkw_x ON fkw (x);
INSERT INTO fkw VALUES (1, 0.3), (2, 9007199254740992), (3, 0.1);
-- @test
SELECT 'exact' AS probe, k FROM fkw WHERE x = 0.3
UNION ALL
SELECT 'rounds', k FROM fkw WHERE x = 0.30000000000000000001
UNION ALL
SELECT 'gt', k FROM fkw WHERE x > 0.10000000000000000001
ORDER BY 1, 2;
