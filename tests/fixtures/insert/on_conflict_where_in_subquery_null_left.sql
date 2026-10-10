-- An `ON CONFLICT DO UPDATE … WHERE` over a NULL `excluded` operand's `IN (SELECT …)` with
-- rows is NULL, so the conflicting row is left as it was.
-- @setup
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
CREATE TABLE u (id INT PRIMARY KEY, v INT);
INSERT INTO u VALUES (2, 5);
-- @profile-lock
-- @test
INSERT INTO u VALUES (2, NULL) ON CONFLICT (id) DO UPDATE SET v = 9 WHERE excluded.v IN (SELECT a FROM r);
SELECT id, v FROM u ORDER BY id;
