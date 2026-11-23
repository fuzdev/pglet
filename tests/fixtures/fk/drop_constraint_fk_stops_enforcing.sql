-- A dropped foreign key no longer checks its child: an orphan row writes, on `INSERT`
-- and on `UPDATE`, and a parent the children reference deletes without `23503`.
-- @profile-lock
-- @setup
CREATE TABLE dcfe_p (id INT PRIMARY KEY);
CREATE TABLE dcfe_c (id INT PRIMARY KEY, pid INT REFERENCES dcfe_p(id));
INSERT INTO dcfe_p VALUES (1), (2);
INSERT INTO dcfe_c VALUES (10, 1), (20, 2);

-- @test
ALTER TABLE dcfe_c DROP CONSTRAINT dcfe_c_pid_fkey;
INSERT INTO dcfe_c VALUES (30, 99);
UPDATE dcfe_c SET pid = 98 WHERE id = 20;
DELETE FROM dcfe_p WHERE id = 1;
SELECT c.id, c.pid, (SELECT count(*) FROM dcfe_p) AS parents FROM dcfe_c c ORDER BY c.id;
