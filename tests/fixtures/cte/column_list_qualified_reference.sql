-- The rename reaches a **qualified** reference and an ORDER BY key, and survives a FROM
-- alias on the reference (which replaces the name, not the columns).
-- @setup
CREATE TABLE clq (a INT);
INSERT INTO clq VALUES (2), (1);
-- @test
WITH c(x) AS (SELECT a FROM clq) SELECT k.x FROM c AS k ORDER BY x;
