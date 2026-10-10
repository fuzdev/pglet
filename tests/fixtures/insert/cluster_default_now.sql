-- @setup
CREATE TABLE t (id INT, created_at TIMESTAMPTZ DEFAULT NOW());
INSERT INTO t (id) VALUES (1);

-- @test
SELECT id, created_at IS NOT NULL AS has_ts FROM t;
