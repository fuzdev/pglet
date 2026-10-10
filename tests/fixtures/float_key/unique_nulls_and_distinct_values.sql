-- What a unique `float8` key accepts: NULLs beside each other, and values that only
-- look alike — `0.1 + 0.2` is not `0.3`.
-- @profile-lock
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
INSERT INTO fku VALUES (1, NULL), (2, NULL), (3, 1.5), (4, 2.5);
INSERT INTO fku VALUES (5, '0.1'::float8 + '0.2'::float8), (6, 0.3);
UPDATE fku SET x = 3.5 WHERE n = 4;
INSERT INTO fku VALUES (7, 2.5);
-- @test
SELECT n, x::text AS shown FROM fku ORDER BY n;
