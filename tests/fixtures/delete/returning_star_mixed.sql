-- @setup
CREATE TABLE t (id INT, name TEXT);
INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c');

-- @test
DELETE FROM t WHERE id = 2 RETURNING *, id + 1 AS next_id;
