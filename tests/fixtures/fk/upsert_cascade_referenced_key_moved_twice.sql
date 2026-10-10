-- An `ON CONFLICT DO UPDATE` that moves a key moves it through the same actions as an
-- `UPDATE`: `gp`'s composite key `(x, y)` takes both parts from one `g` row, so it moves
-- twice, and `gc`, which references it, follows it both times.
-- @setup
CREATE TABLE g (k int PRIMARY KEY);
CREATE TABLE gp (id int PRIMARY KEY, x int REFERENCES g ON UPDATE CASCADE, y int REFERENCES g ON UPDATE CASCADE, UNIQUE (x, y));
CREATE TABLE gc (id int PRIMARY KEY, cx int, cy int, FOREIGN KEY (cx, cy) REFERENCES gp (x, y) ON UPDATE CASCADE);
INSERT INTO g VALUES (1);
INSERT INTO gp VALUES (1, 1, 1);
INSERT INTO gc VALUES (1, 1, 1);
-- @test
-- @profile-lock
INSERT INTO g VALUES (1) ON CONFLICT (k) DO UPDATE SET k = 5;
SELECT gp.x, gp.y, gc.cx, gc.cy FROM gp, gc;
