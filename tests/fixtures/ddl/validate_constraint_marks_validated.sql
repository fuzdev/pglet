-- `VALIDATE CONSTRAINT` once no row violates: the check is reported validated, and `NOT VALID` leaves its definition.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID;
DELETE FROM nv WHERE a < 0;
ALTER TABLE nv VALIDATE CONSTRAINT nv_pos;
SELECT conname, convalidated, pg_get_constraintdef(oid) AS def FROM pg_constraint WHERE conname = 'nv_pos';
