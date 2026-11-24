-- A table constraint written `NOT VALID` in `CREATE TABLE` is validated anyway, as on 18.7.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
CREATE TABLE nv2 (a int, CONSTRAINT nv2_pos CHECK (a > 0) NOT VALID);
SELECT conname, convalidated FROM pg_constraint WHERE conname = 'nv2_pos';
