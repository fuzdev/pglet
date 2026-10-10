-- A `LEFT JOIN … ON` over a NULL operand's `NOT IN (SELECT …)` with rows is NULL, so the
-- left row is kept with no partner.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
-- @test
SELECT t.x, r.a FROM t LEFT JOIN r ON t.x NOT IN (SELECT a FROM r) ORDER BY 2;
