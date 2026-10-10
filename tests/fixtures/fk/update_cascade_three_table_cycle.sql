-- Three tables referencing each other in a ring, `x → y → z → x`, all `ON UPDATE CASCADE`:
-- moving `x`'s key moves `y`'s reference, and the queue ends.
-- @setup
CREATE TABLE x (k int PRIMARY KEY, zk int);
CREATE TABLE y (k int PRIMARY KEY, xk int REFERENCES x ON UPDATE CASCADE);
CREATE TABLE z (k int PRIMARY KEY, yk int REFERENCES y ON UPDATE CASCADE);
INSERT INTO x VALUES (1, 1);
INSERT INTO y VALUES (1, 1);
INSERT INTO z VALUES (1, 1);
ALTER TABLE x ADD FOREIGN KEY (zk) REFERENCES z ON UPDATE CASCADE;
-- @test
-- @profile-lock
UPDATE x SET k = 5;
SELECT 'x' AS t, k, zk AS ref FROM x UNION ALL SELECT 'y', k, xk FROM y UNION ALL SELECT 'z', k, yk FROM z ORDER BY 1;
