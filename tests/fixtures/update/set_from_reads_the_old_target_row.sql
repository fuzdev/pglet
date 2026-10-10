-- `UPDATE … FROM`: a right-hand side reading both the target and the source reads the
-- target's old value, `b = a + s.v` taking the `a` the row held before `a = s.v`.
-- @setup
CREATE TABLE sotf (id INT, a INT, b INT);
INSERT INTO sotf VALUES (1, 10, 0), (2, 20, 0);
CREATE TABLE sotf_src (id INT, v INT);
INSERT INTO sotf_src VALUES (1, 100), (2, 200);

-- @test
UPDATE sotf SET a = s.v, b = a + s.v FROM sotf_src s WHERE s.id = sotf.id
RETURNING sotf.id, sotf.a, sotf.b;
