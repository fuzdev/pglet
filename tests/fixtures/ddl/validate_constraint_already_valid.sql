-- `VALIDATE CONSTRAINT` of a validated check, of a `NOT NULL` and of a foreign key: each succeeds and changes nothing.
-- @setup
CREATE TABLE nv (id int PRIMARY KEY, a int);
INSERT INTO nv VALUES (1, -1), (2, 5);
-- @test
ALTER TABLE nv ADD CONSTRAINT nv_small CHECK (a < 100);
CREATE TABLE nvc (r int REFERENCES nv (id));
ALTER TABLE nv VALIDATE CONSTRAINT nv_small;
ALTER TABLE nv VALIDATE CONSTRAINT nv_id_not_null;
ALTER TABLE nvc VALIDATE CONSTRAINT nvc_r_fkey;
SELECT conname, convalidated FROM pg_constraint WHERE conname IN ('nv_small', 'nvc_r_fkey') ORDER BY 1;
