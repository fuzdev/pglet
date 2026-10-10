-- `JOIN` binds tighter than comma, so USING's left side here is just `t2` (not
-- `t1, t2`). `t1` also carrying `num` therefore does not make the USING left
-- ambiguous — Postgres accepts this, where treating comma and JOIN as one
-- left-deep accumulator wrongly raised 42702. The comma item `t1` cross-joins the
-- `t2 JOIN t3 USING (num)` result.
-- @setup
CREATE TABLE t1 (num INT, name TEXT);
CREATE TABLE t2 (num INT, value TEXT);
CREATE TABLE t3 (num INT, note TEXT);
INSERT INTO t1 VALUES (1, 'a1'), (2, 'a2');
INSERT INTO t2 VALUES (1, 'b1'), (2, 'b2');
INSERT INTO t3 VALUES (1, 'c1'), (2, 'c2');

-- @test
SELECT t1.name, t2.value, t3.note, t3.num
FROM t1, t2 JOIN t3 USING (num)
ORDER BY t1.name, t3.num;
