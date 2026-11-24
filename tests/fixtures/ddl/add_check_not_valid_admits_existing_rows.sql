-- `ADD CONSTRAINT … CHECK … NOT VALID` over a row it rejects: added without reading a row, reported unvalidated, and the row stays.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID;
SELECT n.id, n.a, c.convalidated, pg_get_constraintdef(c.oid) AS def FROM nv n CROSS JOIN pg_constraint c WHERE c.conname = 'nv_pos' ORDER BY n.id;
