-- A self-referencing foreign key drops like any other, and deleting the row its
-- children named sets nothing to NULL afterwards.
-- @profile-lock
-- @setup
CREATE TABLE dcsr (id INT PRIMARY KEY, boss INT REFERENCES dcsr(id) ON DELETE SET NULL);
INSERT INTO dcsr VALUES (1, NULL), (2, 1), (3, 1);

-- @test
ALTER TABLE dcsr DROP CONSTRAINT dcsr_boss_fkey;
DELETE FROM dcsr WHERE id = 1;
SELECT id, boss FROM dcsr ORDER BY id;
