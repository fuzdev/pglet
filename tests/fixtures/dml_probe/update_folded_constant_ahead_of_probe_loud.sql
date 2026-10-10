-- A constant 18.6 folds while planning (`1/0`) is refused before any row there; pglet
-- evaluates it per row, and the write takes no probe that would leave every row out:
-- 22012 on both, where a probe of the absent key would report `UPDATE 0`.
-- @profile-lock
-- @expect-error: division by zero
-- @setup
CREATE TABLE fc (id INT PRIMARY KEY, x INT);
INSERT INTO fc VALUES (1, 5);
-- @test
UPDATE fc SET x = x WHERE 1/0 = 1 AND id = 9;
