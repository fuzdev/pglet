-- The same walk's reverse polarity: the body's own name is gone from the
-- `UPDATE … FROM` scope too, and this one **writes** if it binds.
-- @expect-error: the pre-rename name is not in the UPDATE FROM scope
-- @setup
CREATE TABLE clus (a INT);
CREATE TABLE clusv (a INT);
INSERT INTO clus VALUES (7);
INSERT INTO clusv VALUES (1), (2);
-- @test
WITH c(p) AS (SELECT a FROM clusv) UPDATE clus SET a = c.a FROM c WHERE c.a = 2 RETURNING a;
