-- An unnamed `CHECK … NOT VALID` takes the engine's name, unvalidated.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
ALTER TABLE nv ADD CHECK (a < 100) NOT VALID;
SELECT conname, convalidated FROM pg_constraint WHERE contype = 'c' AND conname LIKE 'nv%' ORDER BY 1;
