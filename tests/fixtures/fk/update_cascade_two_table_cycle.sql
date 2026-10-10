-- Two tables that reference each other's keys `ON UPDATE CASCADE`: moving either key moves
-- the other table's reference, and the queue ends.
-- @setup
CREATE TABLE a (k int PRIMARY KEY, bk int);
CREATE TABLE b (k int PRIMARY KEY, ak int);
INSERT INTO a VALUES (1, 1);
INSERT INTO b VALUES (1, 1);
ALTER TABLE a ADD FOREIGN KEY (bk) REFERENCES b ON UPDATE CASCADE;
ALTER TABLE b ADD FOREIGN KEY (ak) REFERENCES a ON UPDATE CASCADE;
-- @test
-- @profile-lock
UPDATE a SET k = 2;
UPDATE b SET k = 7;
SELECT 'a' AS t, k, bk AS ref FROM a UNION ALL SELECT 'b', k, ak FROM b ORDER BY 1;
