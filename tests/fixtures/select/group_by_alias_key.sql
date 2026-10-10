-- A `GROUP BY` key naming an **output alias** for an expression. `scaled` is no column
-- of `gba`, so Postgres's FROM-column precheck finds nothing and the targetlist search
-- binds the key to `x * 10`. pglet resolved no aliases: the key stayed an unresolvable
-- `scaled` column reference and the projected `x * 10` raised a spurious `42803`.
-- @setup
CREATE TABLE gba (x INT, y INT);
INSERT INTO gba VALUES (1, 10), (1, 20), (2, 30);
-- @test
SELECT x * 10 AS scaled, sum(y) AS total FROM gba GROUP BY scaled ORDER BY 1;
