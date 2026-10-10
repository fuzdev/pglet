-- A `NO ACTION` key reads the rows as the actions before it left them. The first row moves
-- both of its keys; `ON UPDATE CASCADE` on `id` repoints the second row to `10`, so when the
-- `NO ACTION` key on `alt` runs, no row references `alt = 1` any more.
-- @setup
CREATE TABLE t (id int PRIMARY KEY, alt int UNIQUE, par int, FOREIGN KEY (par) REFERENCES t (id) ON UPDATE CASCADE, FOREIGN KEY (par) REFERENCES t (alt) ON UPDATE NO ACTION);
INSERT INTO t VALUES (1, 1, NULL), (2, 2, 1);
-- @test
-- @profile-lock
UPDATE t SET id = 10, alt = 10 WHERE id = 1;
SELECT id, alt, par FROM t ORDER BY id;
