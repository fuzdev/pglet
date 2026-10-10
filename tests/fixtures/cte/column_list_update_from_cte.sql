-- The rename reaches the `UPDATE … FROM` scope, which is built by a **different**
-- relation walk from the SELECT one (`plan_relations` / `push_plan_relations`, not
-- `collect_nonbase_into`) — a reader the SELECT fixtures cannot move.
-- @setup
CREATE TABLE clu (a INT);
CREATE TABLE cluv (a INT);
INSERT INTO clu VALUES (7);
INSERT INTO cluv VALUES (1), (2);
-- @test
WITH c(p) AS (SELECT a FROM cluv) UPDATE clu SET a = c.p FROM c WHERE c.p = 2 RETURNING a;
