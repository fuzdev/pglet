-- @setup
CREATE TABLE da (id INT, x INT);
CREATE TABLE db (id INT, x INT);
INSERT INTO da VALUES (1, 10), (2, 20), (3, 30), (4, 40);
INSERT INTO db VALUES (1, 5), (2, 5), (3, 7), (4, 7);

-- The junk column has to be **the qualified relation's** column, and the output
-- column is deliberately left named `x` so the alternative is live: a bare-name
-- resolver above the projection binds `db.x` to the same-named projected `da.x` and
-- dedups by the wrong column entirely, answering four rows where 18.6 answers two.
-- @test
SELECT DISTINCT ON (db.x) da.x FROM da JOIN db ON da.id = db.id ORDER BY db.x, da.x DESC;
