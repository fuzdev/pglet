-- @setup
CREATE TABLE t (id INT, status TEXT DEFAULT 'active');
INSERT INTO t (id) VALUES (1);

-- @test
SELECT id, status FROM t;
