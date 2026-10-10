-- An UPDATE that leaves a row's unique key unchanged (the row updating itself)
-- is allowed — the row's own index entry is excluded from the conflict check.
-- @profile-lock
-- @setup
CREATE TABLE uq_self (id INT PRIMARY KEY, name TEXT);
INSERT INTO uq_self VALUES (1, 'a'), (2, 'b');
UPDATE uq_self SET id = 1, name = 'a2' WHERE id = 1;

-- @test
SELECT * FROM uq_self ORDER BY id;
