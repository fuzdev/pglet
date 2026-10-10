-- @setup
CREATE TABLE c1 (num INT, name TEXT);
CREATE TABLE c2 (num INT, value TEXT);
CREATE TABLE c3 (num INT, note TEXT);
INSERT INTO c1 VALUES (1, 'one'), (2, 'two'), (3, 'three');
INSERT INTO c2 VALUES (1, 'a'), (2, 'b'), (3, 'c');
INSERT INTO c3 VALUES (1, 'x'), (2, 'y');

-- @test
SELECT * FROM c1 JOIN c2 USING (num) JOIN c3 USING (num) ORDER BY num;
