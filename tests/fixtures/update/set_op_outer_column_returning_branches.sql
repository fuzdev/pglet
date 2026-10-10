-- `RETURNING` reads the updated row's `numeric` column through set operations beside an
-- `integer` constant: each construct is `numeric`.
-- @setup
CREATE TABLE sorb (n NUMERIC, k INT);
INSERT INTO sorb VALUES (1.25, 1);
-- @test
UPDATE sorb SET k = k + 1 RETURNING coalesce((SELECT n EXCEPT SELECT 1), 2) AS c, greatest((SELECT n EXCEPT SELECT 1), 1) AS g, k;
