-- A row an action rewrites a second time checks every one of its keys, changed or not.
-- `g`'s move moves `gp`'s key `(x, y)` twice, so `gc` is repointed twice; between the second
-- repoint and its check, `gp`'s move of `y` cascades `zt`'s key to `5`, whose own repoint
-- of `gc.z` is queued behind that check — which finds `z = 1` gone.
-- @setup
CREATE TABLE g (k int PRIMARY KEY);
CREATE TABLE gp (id int PRIMARY KEY, x int REFERENCES g ON UPDATE CASCADE, y int UNIQUE REFERENCES g ON UPDATE CASCADE, UNIQUE (x, y));
CREATE TABLE gc (id int PRIMARY KEY, cx int, cy int, z int, FOREIGN KEY (cx, cy) REFERENCES gp (x, y) ON UPDATE CASCADE);
CREATE TABLE zt (k int PRIMARY KEY REFERENCES gp (y) ON UPDATE CASCADE);
ALTER TABLE gc ADD FOREIGN KEY (z) REFERENCES zt ON UPDATE CASCADE;
INSERT INTO g VALUES (1);
INSERT INTO gp VALUES (1, 1, 1);
INSERT INTO zt VALUES (1);
INSERT INTO gc VALUES (1, 1, 1, 1);
-- @test
-- @expect-error: the twice-rewritten row's unchanged key is checked
-- @expect-message
-- @profile-lock
UPDATE g SET k = 5;
