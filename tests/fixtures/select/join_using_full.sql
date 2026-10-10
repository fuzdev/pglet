-- @setup
CREATE TABLE t1 (num INT, name TEXT);
CREATE TABLE t2 (num INT, value TEXT);
INSERT INTO t1 VALUES (1, 'one'), (2, 'two'), (3, 'three');
INSERT INTO t2 VALUES (1, 'a'), (2, 'b'), (4, 'd');

-- @test
SELECT * FROM t1 FULL JOIN t2 USING (num) ORDER BY num;
