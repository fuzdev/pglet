-- @setup
CREATE TABLE m1 (a INT, b INT, x TEXT);
CREATE TABLE m2 (b INT, a INT, y TEXT);
INSERT INTO m1 VALUES (1, 10, 'x1'), (2, 20, 'x2'), (3, 30, 'x3');
INSERT INTO m2 VALUES (10, 1, 'y1'), (20, 2, 'y2'), (99, 9, 'y9');

-- @test
SELECT * FROM m1 JOIN m2 USING (a, b) ORDER BY a, b;
