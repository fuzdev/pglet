-- A referenced key that moves twice in one statement moves the rows referencing it twice.
-- `gp`'s composite key `(x, y)` takes both parts from one `g` row through two
-- `ON UPDATE CASCADE` keys, so moving `g` moves the key `(1, 1)` to `(5, 1)` and then to
-- `(5, 5)`, and `gc`, which references it, follows it both times.
-- @setup
CREATE TABLE g (k int PRIMARY KEY);
CREATE TABLE gp (id int PRIMARY KEY, x int REFERENCES g ON UPDATE CASCADE, y int REFERENCES g ON UPDATE CASCADE, UNIQUE (x, y));
CREATE TABLE gc (id int PRIMARY KEY, cx int, cy int, FOREIGN KEY (cx, cy) REFERENCES gp (x, y) ON UPDATE CASCADE);
INSERT INTO g VALUES (1);
INSERT INTO gp VALUES (1, 1, 1);
INSERT INTO gc VALUES (1, 1, 1);
-- @test
-- @profile-lock
UPDATE g SET k = 5;
SELECT gp.x, gp.y, gc.cx, gc.cy FROM gp, gc;
