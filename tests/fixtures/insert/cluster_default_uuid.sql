-- @setup
CREATE TABLE t (id UUID DEFAULT gen_random_uuid(), n INT);
INSERT INTO t (n) VALUES (1);

-- @test
SELECT n, id IS NOT NULL AS has_id FROM t;
