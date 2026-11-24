-- A `VALIDATE CONSTRAINT` taken back with a savepoint: the check is unvalidated again.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID;
DELETE FROM nv WHERE a < 0;
SAVEPOINT s;
ALTER TABLE nv VALIDATE CONSTRAINT nv_pos;
ROLLBACK TO SAVEPOINT s;
SELECT convalidated FROM pg_constraint WHERE conname = 'nv_pos';
