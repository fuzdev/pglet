-- The per-row unique check sees an equality-probed UPDATE's writes in row order: over rows
-- (g, k) inserted (1, 2) then (1, 1) with k UNIQUE, `SET k = k + 1 WHERE g = 1` writes
-- 2 → 3 and then 1 → 2, and succeeds; the other order would write 1 → 2 over the live 2.
-- @profile-lock
-- @setup
CREATE TABLE pu (g INT, k INT UNIQUE);
CREATE INDEX pu_g ON pu (g);
INSERT INTO pu VALUES (1, 2), (1, 1);
-- @test
UPDATE pu SET k = k + 1 WHERE g = 1 RETURNING g, k;
