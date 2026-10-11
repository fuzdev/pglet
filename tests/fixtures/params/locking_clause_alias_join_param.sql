-- An undeclared `$N` compared with an aliased relation's column, under a locking
-- clause over a join. The clause wraps the read in `LockRows`, and the FROM-alias
-- map parameter typing reads must look through it as through any other wrapper
-- that keeps the FROM relations: without that, `lca_a.id`'s qualifier names no
-- relation, `$1` falls back to `text`, and the comparison is `42883 operator does
-- not exist: uuid = text` where 18.6 types `$1` as `uuid` and returns the row.
-- fuz_app's role-grant accept issues this shape (`… JOIN actor act … WHERE
-- rg.actor_id = $1 … FOR SHARE OF rg`).
-- @setup
CREATE TABLE lca_a (id UUID);
CREATE TABLE lca_b (id UUID);
INSERT INTO lca_a VALUES ('80be9f6a-7337-43cf-8027-93e55cdc2331');
INSERT INTO lca_b VALUES ('80be9f6a-7337-43cf-8027-93e55cdc2331');
-- @param $1 uuid 80be9f6a-7337-43cf-8027-93e55cdc2331
-- @test
SELECT a.id FROM lca_a a JOIN lca_b b ON b.id = a.id WHERE a.id = $1 FOR SHARE OF a;
