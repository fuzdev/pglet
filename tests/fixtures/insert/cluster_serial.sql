-- @setup
CREATE TABLE t (id BIGSERIAL, name TEXT);
INSERT INTO t (name) VALUES ('a'), ('b'), ('c');

-- @test
SELECT id, name FROM t ORDER BY id;
