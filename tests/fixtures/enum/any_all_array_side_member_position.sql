-- `op ANY`/`op ALL` with the enum on the **array** side. `eval_any_all` compares the
-- scalar operand against each element, so the comparison is typed by either side — but
-- the walk read the scalar one alone, and `ARRAY[m]` was the operand carrying the
-- identity.
--
-- The control is the same query with the enum on the scalar side
-- (`any_all_scalar_side_member_position`), which agreed on both engines throughout.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');

-- @test
SELECT id, ('ok' > ANY(ARRAY[m])) AS a, ('ok' > ALL(ARRAY[m])) AS b FROM t ORDER BY id;
