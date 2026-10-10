-- An `int8` probe past 2^53 rounds to the `float8` key it compares equal to on 18.6.
-- @profile-lock
-- @setup
CREATE TABLE fkw (k INT, x FLOAT8);
CREATE INDEX fkw_x ON fkw (x);
INSERT INTO fkw VALUES (1, 0.3), (2, 9007199254740992), (3, 0.1);
-- @test
SELECT 'literal' AS probe, k FROM fkw WHERE x = 9007199254740993
UNION ALL
SELECT 'cast', k FROM fkw WHERE x = 9007199254740993::bigint
ORDER BY 1, 2;
