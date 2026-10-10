-- Two-level ON UPDATE CASCADE is transitive: a <- b <- cc. Updating a's key repoints
-- b, and b's changed key in turn repoints cc (the grandchild is not orphaned).
-- @setup
CREATE TABLE outl_a (id INT PRIMARY KEY);
CREATE TABLE outl_b (aid INT PRIMARY KEY REFERENCES outl_a(id) ON UPDATE CASCADE);
CREATE TABLE outl_cc (bid INT PRIMARY KEY REFERENCES outl_b(aid) ON UPDATE CASCADE);
INSERT INTO outl_a VALUES (1);
INSERT INTO outl_b VALUES (1);
INSERT INTO outl_cc VALUES (1);
UPDATE outl_a SET id = 2;

-- @test
SELECT
  (SELECT id FROM outl_a) AS a,
  (SELECT aid FROM outl_b) AS b,
  (SELECT bid FROM outl_cc) AS cc;
