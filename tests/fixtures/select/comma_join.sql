-- @setup
CREATE TABLE t1 (id INT, a TEXT);
CREATE TABLE t2 (id INT, b TEXT);
INSERT INTO t1 VALUES (1, 'x'), (2, 'y');
INSERT INTO t2 VALUES (1, 'p'), (2, 'q'), (3, 'r');

-- @test
SELECT t1.a, t2.b FROM t1, t2 WHERE t1.id = t2.id ORDER BY t1.a, t2.b;
