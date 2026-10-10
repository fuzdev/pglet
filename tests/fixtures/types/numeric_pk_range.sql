-- A NUMERIC primary key with a range query and ordered scan — exercises the
-- order-preserving B-tree key across signs and scales (index acceleration on).
-- @profile-lock
-- @setup
CREATE TABLE t (amount NUMERIC PRIMARY KEY, label TEXT);
INSERT INTO t VALUES (-10.5, 'a'), (0, 'b'), (1.25, 'c'), (100, 'd'), (1000.001, 'e');

-- @test
SELECT amount, label FROM t WHERE amount > -1 AND amount < 500 ORDER BY amount;
