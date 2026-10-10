-- The per-row unique check sees a range-filtered UPDATE's writes in row order. Over UNIQUE
-- rows inserted 2 then 1, `k = k + 1` writes 2 → 3 and then 1 → 2, and succeeds; in the
-- index's key order it would write 1 → 2 over the live 2 and raise 23505.
-- @profile-lock
-- @setup
CREATE TABLE o3 (k INT UNIQUE);
INSERT INTO o3 VALUES (2), (1);
-- @test
UPDATE o3 SET k = k + 1 WHERE k > 0 RETURNING k;
